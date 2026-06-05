[![](https://tokei.rs/b1/github/aegoroff/tinkoff?category=code)](https://github.com/XAMPPRocky/tokei)

# Tinkoff Investment Console Client

A fast and feature-rich console client for Tinkoff Investment API that provides comprehensive portfolio analysis and trading history visualization.

## Features

- 📊 **Portfolio Analysis**: View all your investment positions with detailed profit/loss calculations
- 📈 **Asset Categories**: Separate views for shares, bonds, ETFs, currencies, and futures
- 💰 **Income Tracking**: Track dividends and coupons with dedicated calendar views
- 📅 **Dividend Calendar**: View upcoming dividend payments for your portfolio
- 📋 **Coupon Calendar**: Track bond coupon payments
- 📋 **Trading History**: Detailed history of all trading operations
- 🎨 **Beautiful Tables**: Clean, formatted output with color-coded information
- ⚡ **High Performance**: Optimized with MiMalloc for Linux systems
- 🔒 **Secure**: Uses Tinkoff API v2 with token authentication

## Installation

### Prerequisites

1. Install Rust (1.70 or later)
2. Get your Tinkoff API v2 token from [Tinkoff Investment](https://www.tinkoff.ru/invest/)

### Build and Install

```bash
# Clone the repository
git clone https://github.com/aegoroff/tinkoff.git
cd tinkoff

# Install the application
cargo install --path .
```

## Configuration

Set your Tinkoff API token as an environment variable:

```bash
export TINKOFF_TOKEN_V2="your_api_token_here"
```

Or provide it via command line argument (see usage below).

## Usage

### Basic Commands

```bash
# Get complete portfolio overview
tinkoff a

# Get portfolio shares only
tinkoff s

# Get portfolio bonds only
tinkoff b

# Get portfolio ETFs only
tinkoff e

# Get portfolio currencies only
tinkoff c

# Get portfolio futures only
tinkoff f

# Get trading history for a specific instrument
tinkoff hi <TICKER>

# Get dividend calendar
tinkoff d

# Get coupon calendar
tinkoff p
```

### Command Line Options

```bash
Usage: tinkoff [OPTIONS] [COMMAND]

Commands:
  a     Get all portfolio positions
  s     Get portfolio shares
  b     Get portfolio bonds
  e     Get portfolio ETFs
  c     Get portfolio currencies
  f     Get portfolio futures
  hi    Get trading history for an instrument
  d     Get dividend calendar for portfolio
  p     Get coupon calendar for portfolio bonds
  j     Get combined dividend and coupon calendar
  r     Analyze portfolio risk metrics
  help  Print this message or the help of the given subcommand(s)

Options:
  -t, --token <VALUE>  Tinkoff API v2 token. If not set, TINKOFF_TOKEN_V2 environment variable will be used
  -h, --help           Print help
  -V, --version        Print version
```

### Examples

```bash
# View complete portfolio with detailed breakdown
tinkoff a

# View only shares with aggregate mode (no individual papers)
tinkoff a --aggregate

# Get trading history for Sberbank shares
tinkoff hi SBER

# View dividend calendar
tinkoff d

# View coupon calendar
tinkoff p

# Get combined dividend and coupon calendar
tinkoff j

# Analyze portfolio risk metrics
tinkoff r

# Use custom token
tinkoff -t "your_token" a
```

## Output Format

The application provides rich, formatted output including:

- **Portfolio Summary**: Total balance, current value, and income
- **Asset Breakdown**: Detailed view by asset type (shares, bonds, ETFs, etc.)
- **Profit/Loss**: Current profit/loss with percentage calculations
- **Income Sources**: Dividends, coupons, and other income
- **Trading History**: Detailed operation history with dates, prices, and quantities
- **Dividend Calendar**: Upcoming dividend payments for portfolio instruments
- **Coupon Calendar**: Bond coupon payment schedule

## Project Structure

```
src/
├── main.rs              # CLI application entry point
├── lib.rs               # Library exports and utility functions
├── client.rs            # Tinkoff API client implementation
├── progress.rs          # Progress indicators
├── ux.rs                # Formatting utilities
└── domain/
    ├── calendar.rs      # Dividend and coupon calendars
    ├── money.rs         # Money, Income types
    ├── paper.rs         # Paper, Position, Profit types
    ├── risk.rs          # Risk analysis
    └── display/
        ├── calendar.rs  # Calendar display formatting
        └── risk.rs      # Risk display formatting
```

## Key Components

- **`TinkoffInvestment`**: Main API client for Tinkoff Investment API
- **`Portfolio`**: Container for all portfolio assets
- **`Asset<P>`**: Generic container for different asset types
- **`Paper<P>`**: Individual investment instrument representation
- **`Money`**: Currency-aware monetary value handling
- **`Income`**: Profit/loss calculations with percentage tracking

## Development

### Building from Source

```bash
# Clone and build
git clone https://github.com/aegoroff/tinkoff.git
cd tinkoff
cargo build --release

# Run tests
cargo test
```

### Dependencies

- **tinkoff-invest-api**: Official Tinkoff Investment API client
- **tokio**: Async runtime
- **clap**: Command line argument parsing
- **comfy-table**: Beautiful table formatting
- **indicatif**: Progress indicators
- **color-eyre**: Error handling with colors
- **mimalloc**: High-performance memory allocator (Linux)

## License

MIT License - see [LICENSE](LICENSE) file for details.

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

## Support

For issues and questions, please use the [GitHub Issues](https://github.com/aegoroff/tinkoff/issues) page.
