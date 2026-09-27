[![](https://tokei.rs/b1/github/aegoroff/tinkoff?category=code)](https://github.com/XAMPPRocky/tokei)

# Tinkoff Investment Console Client

A fast and feature-rich console client for Tinkoff Investment API that provides comprehensive portfolio analysis and trading history visualization.

## Features

- 📊 **Portfolio Analysis**: View all your investment positions with detailed profit/loss calculations
- 📈 **Asset Categories**: Separate views for shares, bonds, ETFs, currencies, and futures
- 💰 **Income Tracking**: Track dividends and coupons with dedicated calendar views
- 📅 **Dividend Calendar**: View upcoming dividend payments for your portfolio
- 📋 **Bond Payments Calendar**: Coupons, amortizations and maturities of portfolio bonds
- 💵 **Passive Income Forecast**: Coupons and dividends expected by month within a year and the current portfolio yield
- 📐 **Returns**: Annual return (XIRR) per position, asset type and portfolio; yield to maturity and to offer, duration and rate sensitivity for bonds
- 📋 **Trading History**: Detailed history of all trading operations
- 🧾 **Taxes**: Dividends, coupons and taxes withheld by the broker by year
- 🏁 **Benchmarks**: Returns compared with the same payments invested into IMOEX, MCFTR, RGBI and RGBITR indices
- 🔭 **Share Analytics**: Analyst consensus forecasts and fundamentals (P/E, P/B, EV/EBITDA, ROE, dividend yield) of portfolio shares
- 🎯 **Risk Analysis**: Portfolio risk metrics, currency and sector allocation, rebalancing recommendations
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

# Get bond payments calendar (coupons, amortizations, maturities) for the next year
tinkoff p

# ... or for the next three years
tinkoff p --days 1095

# Get combined dividend and bond payments calendar
tinkoff j

# Forecast passive income for a year and the current portfolio yield
tinkoff in

# Income and taxes withheld by year
tinkoff tx

# Compare returns with market indices
tinkoff bm

# Analyze portfolio risk metrics
tinkoff r

# Analyst forecasts and fundamentals of portfolio shares
tinkoff an

# Risk metrics plus rebalancing recommendations to your target allocation
tinkoff r --target bonds=60,shares=30,etfs=10

# ... or to a preset: conservative (60/30/5/5/0) or balanced (40/40/10/5/5)
tinkoff r --target balanced
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
  p     Get bond payments calendar: coupons, amortizations and maturities
  j     Get combined dividend and bond payments calendar
  r     Analyze portfolio risk metrics
  ac    List accounts
  an    Get analyst forecasts and fundamentals of portfolio shares
  in    Forecast passive income for a year: coupons, dividends and current yield
  tx    Get income and taxes withheld by year
  bm    Compare returns of the portfolio with IMOEX, MCFTR, RGBI and RGBITR indices
  help  Print this message or the help of the given subcommand(s)

Options:
  -t, --token <VALUE>      Tinkoff API v2 token. If not set, TINKOFF_TOKEN_V2 environment variable will be used
      --account <TYPE>     Account type: tinkoff (broker, default), iis, invest-box, invest-fund.
                           Selects the only open account of this type
      --account-id <ID>    Account ID (see the ac command); takes precedence over --account
  -h, --help               Print help
  -V, --version            Print version
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

# View bond payments calendar
tinkoff p

# Get combined dividend and bond payments calendar
tinkoff j

# Analyze portfolio risk metrics
tinkoff r

# List accounts, then pick one by ID when several accounts have the same type
tinkoff ac
tinkoff --account-id 2000000000 a

# Portfolio of the individual investment account
tinkoff --account iis a

# Use custom token
tinkoff -t "your_token" a
```

## Output Format

The application provides rich, formatted output including:

- **Portfolio Summary**: Total balance, current value, and income
- **Asset Breakdown**: Detailed view by asset type (shares, bonds, ETFs, etc.)
- **Profit/Loss**: Current profit/loss with percentage calculations
- **Annual Return (XIRR)**: Return of all payments of a position (buys, sells, dividends, coupons, taxes, fees) plus its current value, per year; also for asset types and the whole portfolio
- **Daily Change**: Change of position, asset type and portfolio value since the previous trading day, as calculated by the broker
- **Blocked Positions**: Amount reserved by active orders and exchange blocks are shown in paper cards
- **Bond Details**: Maturity date, next offer date, yield to maturity and yield to offer (not shown when future coupons are not known yet, e.g. floating ones)
- **Income Sources**: Dividends, coupons, and other income
- **Trading History**: Detailed operation history with dates, prices, and quantities
- **Dividend Calendar**: Upcoming dividend payments for portfolio instruments
- **Bond Payments Calendar**: Coupons, amortizations and maturities within `--days` (365 by default); offers are not included as they are not guaranteed payments
- **Share Analytics**: Consensus recommendation, 12 months target price, upside and target range of investment houses; P/E, P/B, EV/EBITDA, Net debt/EBITDA, ROE, 12 months dividend yield and beta (`n/a` when not provided)
- **Risk Analysis**: Asset allocation, risk metrics, and rebalancing recommendations to the target set with `--target`: a preset (`conservative`, `balanced`) or percents (asset types: bonds, shares, etfs, currencies, futures; omitted ones are 0%, the sum must be 100%)

## Project Structure

```
src/
├── main.rs              # CLI application entry point
├── lib.rs               # Library exports and utility functions
├── client.rs            # Tinkoff API client implementation
├── progress.rs          # Progress indicators
├── ux.rs                # Formatting utilities
└── domain/
    ├── analytics.rs     # Analyst forecasts and fundamentals
    ├── bond.rs          # Bond events, yield to maturity and to offer
    ├── calendar.rs      # Dividend and bond payments calendars
    ├── money.rs         # Money, Income types
    ├── paper.rs         # Paper, Position, Profit types
    ├── risk.rs          # Risk analysis
    ├── xirr.rs          # Annual return of irregular cash flows
    └── display/
        ├── analytics.rs # Forecasts and fundamentals tables
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

- **t-invest-sdk**: T-Invest API client
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
