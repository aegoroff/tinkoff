[![](https://tokei.rs/b1/github/aegoroff/tinkoff?category=code)](https://github.com/XAMPPRocky/tokei)

# Tinkoff
Tinkoff investments console client

# Installation
Install Rust, then go to sources root and then run:
```shell
cargo install --path .
```
# Usage
```
Usage: tinkoff [OPTIONS] [COMMAND]

Commands:
  a     Get all portfolio
  s     Get portfolio shares
  b     Get portfolio bonds
  e     Get portfolio etfs
  c     Get portfolio currencies
  f     Get portfolio futures
  hi    Get an instrument history
  help  Print this message or the help of the given subcommand(s)

Options:
  -t, --token <VALUE>  Tinkoff API v2 token. If not set TINKOFF_TOKEN_V2 environment variable will be used
  -h, --help           Print help
  -V, --version        Print version
```
