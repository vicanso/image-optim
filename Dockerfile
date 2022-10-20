FROM rust:alpine as builder

COPY . /image-optim

RUN apk update \
  && apk add git make build-base nasm openssl-dev cmake pkgconfig aom-libs perl \
  && cd /image-optim \
  && cargo build --release

FROM alpine 

EXPOSE 3000 

# tzdata 安装所有时区配置或可根据需要只添加所需时区

RUN addgroup -g 1000 rust \
  && adduser -u 1000 -G rust -s /bin/sh -D rust \
  && apk add --no-cache ca-certificates tzdata

COPY --from=builder /image-optim/target/release/image-optim /usr/local/bin/image-optim
COPY --from=builder /image-optim/entrypoint.sh /entrypoint.sh

USER rust

WORKDIR /home/rust

HEALTHCHECK --timeout=10s --interval=10s CMD [ "wget", "http://127.0.0.1:3000/ping", "-q", "-O", "-"]


CMD ["image-optim"]

ENTRYPOINT ["/entrypoint.sh"]