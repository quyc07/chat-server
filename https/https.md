# 配置https

## nginx

### 检查nginx是否支持ssl模块

```shell
nginx -V
```

```
root@11eea2a381e9:/# nginx -V
nginx version: nginx/1.25.3
built by gcc 12.2.0 (Debian 12.2.0-14) 
built with OpenSSL 3.0.9 30 May 2023 (running with OpenSSL 3.0.11 19 Sep 2023)
TLS SNI support enabled
configure arguments: --prefix=/etc/nginx --sbin-path=/usr/sbin/nginx --modules-path=/usr/lib/nginx/modules --conf-path=/etc/nginx/nginx.conf --error-log-path=/var/log/nginx/error.log --http-log-path=/var/log/nginx/access.log --pid-path=/var/run/nginx.pid --lock-path=/var/run/nginx.lock --http-client-body-temp-path=/var/cache/nginx/client_temp --http-proxy-temp-path=/var/cache/nginx/proxy_temp --http-fastcgi-temp-path=/var/cache/nginx/fastcgi_temp --http-uwsgi-temp-path=/var/cache/nginx/uwsgi_temp --http-scgi-temp-path=/var/cache/nginx/scgi_temp --user=nginx --group=nginx --with-compat --with-file-aio --with-threads --with-http_addition_module --with-http_auth_request_module --with-http_dav_module --with-http_flv_module --with-http_gunzip_module --with-http_gzip_static_module --with-http_mp4_module --with-http_random_index_module --with-http_realip_module --with-http_secure_link_module --with-http_slice_module --with-http_ssl_module --with-http_stub_status_module --with-http_sub_module --with-http_v2_module --with-http_v3_module --with-mail --with-mail_ssl_module --with-stream --with-stream_realip_module --with-stream_ssl_module --with-stream_ssl_preread_module --with-cc-opt='-g -O2 -ffile-prefix-map=/data/builder/debuild/nginx-1.25.3/debian/debuild-base/nginx-1.25.3=. -fstack-protector-strong -Wformat -Werror=format-security -Wp,-D_FORTIFY_SOURCE=2 -fPIC' --with-ld-opt='-Wl,-z,relro -Wl,-z,now -Wl,--as-needed -pie'
```

### 生成自己的ssl证书

#### 生成RSA秘钥

```shell
openssl genrsa -out server.key 2048
```

#### 生成一个证书请求

```shell
openssl req -new -key server.key -out server.csr
```

#### 生成一个自己签发的证书

```shell
openssl x509 -req -days 365 -in server.csr -signkey server.key -out server.crt
```

### 创建一个文件夹用于存放上述生成的证书文件

```shell
mkdir -p /volume/nginx/ssl
mv server.key /volume/nginx/ssl
mv server.csr /volume/nginx/ssl
mv server.crt /volume/nginx/ssl
```

### 修改nginx.conf以支持ssl

```
    server {
        listen 443 ssl;
        server_name dev.demo.com;

        #证书文件
        ssl_certificate      /opt/nginx/ssl_key/server.crt;
        #私钥文件
        ssl_certificate_key  /opt/nginx/ssl_key/server.key;
        ssl_session_cache    shared:SSL:1m;
        ssl_session_timeout  5m;
        ssl_ciphers          HIGH:!aNULL:!MD5;
        ssl_prefer_server_ciphers  on;


        location / {
            root   /usr/share/nginx/html;
            index  index.html index.htm;
        }

    }

    server {
        listen 80;
        server_name dev.demo.com;
        #将http请求转成https
        rewrite ^(.*)$ https://dev.demo.com permanent;
    }
```

## SpringBoot

### 将之前生成的证书转换成pkcs12格式

```shell
openssl pkcs12 -export -clcerts -in server.crt -inkey server.key -out server.pkcs12
```

### 修改 application.yml 以支持ssl

```yaml
server:
  port: 8080
  ssl:
    key-store: classpath:server.pkcs12 # 将pkcs12文件放在resources目录下
    key-store-password: 123456 # 该密码是在生成证书时创建的
```
