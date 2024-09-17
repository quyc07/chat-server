FROM rust:latest as builder
WORKDIR /usr/src/chat-server
COPY . .
RUN cargo install --path .

FROM debian:bullseye-slim
# TODO 无法获取 extra-runtime-dependencies
RUN apt-get update && apt-get install -y extra-runtime-dependencies && rm -rf /var/lib/apt/lists/*
COPY --from=builder /usr/local/cargo/bin/chat-server /usr/local/bin/chat-server
CMD ["chat-server"]
