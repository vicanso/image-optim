FROM rust:1.90 as builder

COPY . /image-optim

RUN apt-get update
RUN apt-get install -y cmake nasm
RUN rustup target list --installed
RUN cd /image-optim \
  && cargo build --release

FROM ubuntu

EXPOSE 3000

COPY --from=builder /image-optim/target/release/image-optim /usr/local/bin/image-optim
COPY --from=builder /image-optim/entrypoint.sh /entrypoint.sh

ENV RUST_ENV=production

USER ubuntu

WORKDIR /home/ubuntu

HEALTHCHECK --timeout=10s --interval=10s CMD [ "wget", "http://127.0.0.1:3000/ping", "-q", "-O", "-"]

CMD ["image-optim"]

ENTRYPOINT ["/entrypoint.sh"]
