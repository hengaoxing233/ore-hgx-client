# ore-hgx-client
修改于：https://github.com/Kriptikz/ore-hq-client

## linux 运行命令（绑定核心）
```taskset -c 0-103 ./ore-hgx-client1 --url http://192.168.1.101:8989 mine --cores 104```
```taskset -c 104-207 ./ore-hgx-client1 --url http://192.168.1.101:8989 mine --cores 104```

## 服务端
### [https://github.com/hengaoxing233/ore-hgx-server](https://github.com/hengaoxing233/ore-hgx-server)
