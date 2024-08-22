use std::{ops::Range, sync::Arc, time::{Duration, Instant}};
use rayon::prelude::*;
use clap::{arg, Parser};
use drillx::{DrillxError, equix, Hash, seed};
use futures_util::SinkExt;
use base64::prelude::*;
use serde_json::Value;
use core_affinity;
use drillx::equix::SolutionArray;
use tracing::{error, info};
use sha3::Digest;
use crate::log::init_log;

#[derive(Debug)]
pub enum ServerMessage {
    StartMining([u8; 32], Range<u64>, u64)
}

#[derive(Debug, Parser)]
pub struct MineArgs {
    #[arg(
        long,
        value_name = "CORES_COUNT",
        help = "The number of CPU cores to allocate to mining",
        global = true
    )]
    cores: Option<usize>,
}

fn create_shared_client() -> Arc<reqwest::blocking::Client> {
    Arc::new(
        reqwest::blocking::Client::builder()
            .http1_title_case_headers()
            .danger_accept_invalid_certs(true)
            .connection_verbose(true)
            .timeout(Duration::from_secs(1))
            .build()
            .unwrap(),
    )
}

fn array_to_base64(data: &[u8; 32]) -> String {
    base64::encode(data)
}

fn base64_to_array(base64_string: &str) -> Result<[u8; 32], &'static str> {
    let decoded_bytes = base64::decode(base64_string).map_err(|_| "Invalid Base64 input")?;
    if decoded_bytes.len() != 32 {
        return Err("Decoded byte length is not 32");
    }
    let array: [u8; 32] = decoded_bytes.as_slice().try_into().map_err(|_| "Decoded byte length is not 32")?;

    Ok(array)
}

fn u8_16_to_base64(data: [u8; 16]) -> String {
    base64::encode(&data)
}

fn base64_to_u8_16(encoded: &str) -> Result<[u8; 16], base64::DecodeError> {
    let decoded = base64::decode(encoded)?;
    let mut array = [0u8; 16];
    array.copy_from_slice(&decoded);
    Ok(array)
}

fn u8_8_to_base64(data: [u8; 8]) -> String {
    base64::encode(&data)
}

fn base64_to_u8_8(encoded: &str) -> Result<[u8; 8], base64::DecodeError> {
    let decoded = base64::decode(encoded)?;
    let mut array = [0u8; 8];
    array.copy_from_slice(&decoded);
    Ok(array)
}

pub async fn mine(args: MineArgs, url: String) {
    init_log();
    let client = create_shared_client();
    let server_url = url.clone();
    info!("服务端url:{}", server_url);
    let cores = args.cores.unwrap_or_else(|| num_cpus::get());

    let mut challenge_str = String::new();
    let mut server_pubkey = String::new();
    let mut min_difficulty = 0;
    let mut nonce_start = 0;
    let mut nonce_end = 0;
    loop {
        info!("----------------------获取任务-----------------------");
        let res = client.get(format!("{server_url}/getchallenge"))
            .header("Content-Type", "application/json")
            .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/127.0.0.0 Safari/537.36 Edg/127.0.0.0")
            .send();
        match res {
            Ok(response) => match response.text() {
                Ok(text) => {
                    info!("获取任务响应内容: {}", text);
                    if let Ok(json) = serde_json::from_str::<Value>(&text) {
                        if let Some(ok) = json["code"].as_u64() {
                            if ok == 1 {
                                challenge_str = json["challenge"].as_str().unwrap_or_default().to_string();
                                server_pubkey = json["pubkey"].as_str().unwrap_or_default().to_string();
                                min_difficulty = json["min_difficulty"].as_u64().unwrap_or_default() as u32;
                                nonce_start = json["nonce_start"].as_u64().unwrap_or_default();
                                nonce_end = json["nonce_end"].as_u64().unwrap_or_default();
                            } else {
                                error!("获取任务失败");
                                tokio::time::sleep(Duration::from_secs(1)).await;
                                continue;
                            }
                        }
                    }
                },
                Err(e) => {
                    error!("获取任务失败: {:?}", e);
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    continue;
                }
            },
            Err(e) => {
                error!("获取任务失败: {:?}", e);
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
            }
        }

        info!("开始挖矿...Nonce range: {} - {}，challenge: {}", nonce_start, nonce_end, challenge_str);
        let mut challenge = [0;32];
        match base64_to_array(&challenge_str) {
            Ok(ret) => challenge = ret,
            Err(_) => {
                continue
            }
        }
        let challenge = base64_to_array(&challenge_str).unwrap();
        let hash_timer = Instant::now();

        let core_ids = Arc::new(core_affinity::get_core_ids().unwrap());

        let total_nonce = nonce_end - nonce_start; // 总的nonce范围
        let step = total_nonce / cores as u64; // 每个核心需要处理的nonce数量
        let remainder = total_nonce % cores as u64; // 处理不能被均分的nonce部分

        let handles = (0..cores).map(|i| {
            let challenge = challenge.clone(); // 确保 challenge 被正确克隆到每个线程中
            let core_ids = Arc::clone(&core_ids); // 克隆 Arc 以避免移动
            std::thread::spawn(move || {
                let mut memory = equix::SolverMemory::new();
                if let Some(core_id) = core_ids.get(i) {
                    core_affinity::set_for_current(*core_id);
                }
                // 为每个核心分配 nonce 范围
                let start_nonce = nonce_start + i as u64 * step + if i < remainder as usize { i as u64 } else { remainder };
                let end_nonce = start_nonce + step + if i < remainder as usize { 1 } else { 0 }; // 保证范围覆盖

                let mut best_nonce = start_nonce;
                let mut best_difficulty = 0;
                let mut best_hash = drillx::Hash::default();
                let mut total_hashes: u64 = 0;

                let hash_timer = Instant::now(); // 计时器开始
                let mut nonce = start_nonce;

                // 挖矿循环，直到达到nonce范围或超时
                while nonce < end_nonce && hash_timer.elapsed().as_secs() < 10 {
                    // Create hash
                    for hx in  get_hashes_with_memory(&mut memory, &challenge, &nonce.to_le_bytes()) {
                        total_hashes += 1;
                        let difficulty = hx.difficulty();
                        if difficulty.gt(&best_difficulty) {
                            best_nonce = nonce;
                            best_difficulty = difficulty;
                            best_hash = hx;
                        }
                    }

                    // Increment nonce
                    nonce += 1;
                }

                Some((best_nonce, best_difficulty, best_hash, total_hashes))
            })
        }).collect::<Vec<_>>();

        let mut best_nonce: u64 = 0;
        let mut best_difficulty = 0;
        let mut best_hash = drillx::Hash::default();
        let mut total_nonces_checked = 0;
        for h in handles {
            if let Ok(Some((nonce, difficulty, hash, nonces_checked))) = h.join() {
                total_nonces_checked += nonces_checked;
                if difficulty > best_difficulty {
                    best_difficulty = difficulty;
                    best_nonce = nonce;
                    best_hash = hash;
                }
            }
        }

        let hash_time = hash_timer.elapsed();
        info!("找到最佳难度: {}", best_difficulty);
        info!("Processed: {}", total_nonces_checked);
        info!("Hash time: {:?}", hash_time);

        if best_difficulty < min_difficulty {
            info!("难度太低，丢弃，重新计算");
            continue;
        }
        info!("----------------------提交任务-----------------------");
        let res = client.post(format!("{server_url}/setsolution"))
            .header("Accept", "application/json, text/plain, */*")
            .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/127.0.0.0 Safari/537.36 Edg/127.0.0.0")
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({
                "challenge": challenge_str,
                "d": u8_16_to_base64(best_hash.d),
                "n": u8_8_to_base64(best_nonce.to_le_bytes()),
                "difficulty": best_difficulty,
                "pubkey": server_pubkey,
            }))
            .send();

        match res {
            Ok(response) => match response.text() {
                Ok(text) => {
                    info!("提交哈希响应内容: {}", text);
                    if let Ok(json) = serde_json::from_str::<Value>(&text) {
                        if let Some(ok) = json["code"].as_u64() {
                            if ok == 1 {
                                info!("提交哈希成功");
                            } else {
                                error!("提交哈希失败");
                            }
                        }
                    }
                },
                Err(e) => {
                    error!("提交哈希失败: {:?}", e);
                }
            },
            Err(e) => error!("提交哈希失败: {:?}", e),
        }

    }
}
#[inline(always)]
fn sorted(mut digest: [u8; 16]) -> [u8; 16] {
    unsafe {
        let u16_slice: &mut [u16; 8] = core::mem::transmute(&mut digest);
        u16_slice.sort_unstable();
        digest
    }
}
#[cfg(not(feature = "solana"))]
#[inline(always)]
fn hashv(digest: &[u8; 16], nonce: &[u8; 8]) -> [u8; 32] {
    let mut hasher = sha3::Keccak256::new();
    hasher.update(&sorted(*digest));
    hasher.update(nonce);
    hasher.finalize().into()
}
pub fn get_hashes_with_memory(
    memory: &mut equix::SolverMemory,
    challenge: &[u8; 32],
    nonce: &[u8; 8],
) -> Vec<Hash> {
    let mut hashes: Vec<Hash> = Vec::with_capacity(7);
    if let Ok(solutions) = get_digests_with_memory(memory, challenge, nonce) {
        for solution in solutions {
            let digest = solution.to_bytes();
            hashes.push(Hash {
                d: digest,
                h: hashv(&digest, nonce),
            });
        }
    }

    hashes
}
#[inline(always)]
fn get_digests_with_memory(
    memory: &mut equix::SolverMemory,
    challenge: &[u8; 32],
    nonce: &[u8; 8],
) -> Result<SolutionArray, DrillxError> {
    let seed = seed(challenge, nonce);
    let equix = equix::EquiXBuilder::new()
        .runtime(equix::RuntimeOption::TryCompile)
        .build(&seed)
        .map_err(|_| DrillxError::BadEquix)?;
    Ok(equix.solve_with_memory(memory))
}
