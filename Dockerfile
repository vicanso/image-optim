FROM rust:1.95 as builder

COPY . /image-optim

RUN apt-get update
RUN apt-get install -y cmake nasm curl ca-certificates --no-install-recommends
RUN rustup target list --installed
RUN curl -L https://github.com/vicanso/http-stat-rs/releases/latest/download/httpstat-linux-musl-$(uname -m).tar.gz | tar -xzf -
  RUN mv httpstat /usr/local/bin/
RUN cd /image-optim \
  && cargo build --release

FROM debian:trixie-slim

EXPOSE 3000

# reqwest+rustls (opendal's HTTPS/S3 backend) panics on Client::new() if the
# system trust store is empty, and trixie-slim ships no CA bundle. Instead of
# running apt here — which permanently bakes ~1.6 MiB of dpkg/debconf cruft into
# the layer (the rewritten /var/lib/dpkg/status DB can't be deleted) — copy the
# CA bundle the builder stage already generated. No package manager runs in the
# runtime stage, so there is no apt/dpkg waste to clean. reqwest+rustls reads
# this path (openssl-probe's default SSL_CERT_FILE).
COPY --from=builder /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt

# Service account with /bin/false to block login; `-m` still creates a home
# dir so `docker exec -it <container> bash` (invoked explicitly) has a HOME.
RUN useradd -r -m -s /bin/false rust

COPY --from=builder --chown=rust:rust --chmod=755 /image-optim/target/release/image-optim /usr/local/bin/image-optim
COPY --from=builder --chown=rust:rust --chmod=755 /image-optim/entrypoint.sh /entrypoint.sh
COPY --from=builder --chown=rust:rust --chmod=755 /usr/local/bin/httpstat /usr/local/bin/httpstat

ENV RUST_ENV=production

USER rust

WORKDIR /home/rust

HEALTHCHECK --timeout=10s --interval=10s CMD [ "httpstat", "http://127.0.0.1:3000/ping", "-s"]

CMD ["image-optim"]

ENTRYPOINT ["/entrypoint.sh"]
