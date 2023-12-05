FROM rust:slim AS builder
SHELL ["/bin/bash", "-uo", "pipefail", "-c"]
RUN apt-get update && apt-get install -y musl-tools musl-dev && rm -rf /var/lib/apt/lists/*

COPY . /opt/wordler
WORKDIR /opt/wordler

ENV TARGET x86_64-unknown-linux-musl
RUN rustup target add "$TARGET"
RUN cargo build --release --locked --target "$TARGET" \
    && mv target/"$TARGET"/release/wordler . \
    && strip wordler

FROM gcr.io/distroless/static
COPY --from=builder /opt/wordler/wordler /opt/wordler/wordler
COPY ./data/corpus.txt /opt/wordler/data/corpus.txt
WORKDIR /opt/wordler
EXPOSE 8080
ENTRYPOINT ["/opt/wordler/wordler"]
