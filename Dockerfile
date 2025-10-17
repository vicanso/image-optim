FROM rust:1.90 as builder

COPY . /image-optim

RUN apt-get update
RUN apt-get install -y cmake nasm curl --no-install-recommends
RUN rustup target list --installed
RUN curl -L https://github.com/vicanso/http-stat-rs/releases/latest/download/httpstat-linux-musl-$(uname -m).tar.gz | tar -xzf -
  RUN mv httpstat /usr/local/bin/
RUN cd /image-optim \
  && cargo build --release

FROM ubuntu:24.04

EXPOSE 3000

COPY --from=builder /image-optim/target/release/image-optim /usr/local/bin/image-optim
COPY --from=builder /image-optim/entrypoint.sh /entrypoint.sh
COPY --from=builder /usr/local/bin/httpstat /usr/local/bin/httpstat

ENV RUST_ENV=production

USER ubuntu

WORKDIR /home/ubuntu

HEALTHCHECK --timeout=10s --interval=10s CMD [ "wget", "http://127.0.0.1:3000/ping", "-q", "-O", "-"]

CMD ["image-optim"]

ENTRYPOINT ["/entrypoint.sh"]
