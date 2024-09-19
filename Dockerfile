FROM rust:alpine AS builder
RUN apk add -q --no-cache build-base openssl-dev curl
ENV RUSTFLAGS="-C target-feature=-crt-static"
WORKDIR /usr/src/chat-server
COPY . .
RUN cargo build --release

# 使用Alpine作为最终镜像
FROM alpine:latest
RUN apk add -q --no-cache libgcc curl
WORKDIR /app
# 复制构建的二进制文件
COPY --from=builder /usr/src/chat-server/target/release/chat-server /app/
CMD ["./chat-server"]

