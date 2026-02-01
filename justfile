test:
  cargo nextest run --workspace --all-features

install:
  cargo install --path . --target x86_64-unknown-linux-musl
