FROM rust:1.69.0 as builder
COPY .. /ephemera
WORKDIR /ephemera
RUN cargo build --release

FROM docker.io/library/ubuntu:20.04

RUN apt-get update && \
	DEBIAN_FRONTEND=noninteractive apt-get upgrade -y && \
	DEBIAN_FRONTEND=noninteractive apt-get install -y \
		libssl1.1


COPY --from=builder /ephemera/target/release/ephemera /usr/local/bin

ENV RUST_BACKTRACE full
ENV RUST_LOG "debug"
ENV EPHEMERA_CONFIG "ephemera.toml"
ENV PEERS_CONFIG "peers.toml"

EXPOSE 7000

ENTRYPOINT /usr/local/bin/ephemera run-node --config-file $EPHEMERA_CONFIG --peers-config $PEERS_CONFIG
