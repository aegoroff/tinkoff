# Agent Rules for Tinkoff Investment Console Client

## Project Overview

This is a Rust console client for Tinkoff Investment API that provides portfolio analysis and trading history visualization.

## Code Style Guidelines

### General Rust Style

- Follow standard Rust naming conventions:
  - `snake_case` for functions, variables, and modules
  - `PascalCase` for types, structs, enums, and traits
  - `SCREAMING_SNAKE_CASE` for constants
- Use `&str` for string parameters unless ownership is needed
- Prefer `Option<T>` and `Result<T, E>` over null or exceptions
- Use `#[must_use]` for functions whose return values should not be ignored

### Error Handling

- Use `color_eyre::eyre::Result<T>` for application-level errors
- Always provide context with `.wrap_err_with()` or `.map_err()`
- Avoid `unwrap()` in production code; use `?` operator or proper error handling
- Use `eyre::eyre!()` for creating custom errors

### Async Patterns

- Use `tokio` as the async runtime
- For parallel operations with concurrency limits, use `tokio::sync::Semaphore`
- Use `JoinSet` for spawning multiple async tasks
- Always handle task results: match on `JoinSet::join_next()` results
- Clone `Arc<T>` for shared state across tasks

### Common Patterns

#### Retry Logic

Wrap raw API calls with `with_retry` in the `InvestApi` implementation (`src/client.rs`);
reports never retry themselves. It retries only transient failures
(`is_transient`: gRPC `Unavailable`, `ResourceExhausted`, `DeadlineExceeded`, `Aborted`,
`Internal`, `Unknown` and transport errors) with exponential backoff, waits for
`x-ratelimit-reset` on `ResourceExhausted`, and returns other errors immediately.
Keep `tonic::Status` in the error chain (`.wrap_err(...)`, not `eyre!("{e:?}")`),
otherwise errors cannot be classified.

```rust
async fn dividends(&self, figi: &str) -> color_eyre::Result<Vec<Dividend>> {
    with_retry(|| self.get_dividends_for_figi(figi.to_string())).await
}
```

#### Parallel Fetching with Concurrency Limit

```rust
let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_REQUESTS));
let mut set = JoinSet::new();

for item in items {
    let permit = semaphore.clone().acquire_owned().await.unwrap();
    let item = item.clone();
    set.spawn(async move {
        let _permit = permit;
        fetch(item).await
    });
}

while let Some(res) = set.join_next().await {
    match res {
        Ok(result) => { /* handle result */ }
        Err(e) => eprintln!("Task failed: {e}"),
    }
}
```

#### Macros

Do not add new macros. Reduce boilerplate with generic functions, fn pointers, closures
or traits instead. The only existing macro is `impl_portfolio_aggregator!`
(`src/domain/portfolio.rs`).

### Documentation

- Use rustdoc comments (`///`) for public APIs
- Include `# Errors`, `# Panics`, `# Examples` sections where applicable
- Write doc tests for utility functions
- Keep inline comments concise and informative

### Testing

- Use `rstest` for parameterized tests
- Place tests in the same file under `#[cfg(test)] mod tests`
- Follow Arrange-Act-Assert pattern
- Test edge cases: zero values, negative values, None cases

### File Organization

```
src/
├── main.rs            # CLI entry point, command handling
├── lib.rs             # Library exports, API value conversions
├── api.rs             # InvestApi trait: the seam between reports and the T-Invest API
├── client.rs          # gRPC adapter of InvestApi: transport, retries, API → domain mapping
├── reports.rs         # Reports<A: InvestApi>: use cases, tested with an in-memory FakeApi
├── progress.rs        # Progress indicators
├── ux.rs              # Table formatting, number formatting
└── domain/
    ├── mod.rs         # Re-exports, NumberRange trait
    ├── account.rs     # Accounts, their types and statuses
    ├── money.rs       # Money and Income
    ├── operation.rs   # Account operations and what their kinds mean for income and taxes
    ├── paper.rs       # Paper, Position, Instrument, Profit kinds
    ├── portfolio.rs   # Portfolio and Asset containers, LoadedPaper
    ├── analytics.rs   # Analyst forecasts and fundamentals of shares
    ├── benchmark.rs   # Returns compared with market indices
    ├── bond.rs        # Bond events, YTM and yield to offer
    ├── calendar.rs    # Dividend and bond payments calendars
    ├── history.rs     # Instrument operations history
    ├── income.rs      # Passive income forecast and portfolio yield
    ├── fx.rs          # Conversion to RUB at daily rates behind the DailyCloses seam
    ├── risk.rs        # Risk analysis, target allocation, rebalancing
    ├── screener.rs    # Bond screener: market bonds by yield to worst, horizon and risk
    ├── tax.rs         # Income and taxes withheld by year
    ├── lots.rs        # FIFO tax lots, tax on sale and long-term holding exemption
    ├── trades.rs      # Rebalancing orders in whole lots, the least taxed sells first
    ├── xirr.rs        # XIRR solver on Decimal
    └── display/       # Display implementations for domain types
```

## Important Conventions

### Trait Usage

- `Profit` trait for asset profit types (DividentProfit, CouponProfit, NoneProfit)
- `NumberRange` trait for checking negative/zero values
- `Display` trait for formatted output

### Money Handling

- Always use `rust_decimal::Decimal` for financial calculations
- Use `iso_currency::Currency` for currency types
- Never use `f64` or `f32` for money values

### API Client Patterns

- Use cases live in `Reports` and reach the API only through `InvestApi`
- `InvestApi` methods of `TinkoffInvestment` retry; its private `get_*` methods are raw API calls
- Test use cases with `FakeApi` (`src/reports.rs` tests), not with the network
- `Reports` and `InvestApi` adapters are cheap to clone for spawned tasks

## CLI Guidelines

- Use `clap` for argument parsing
- Short commands: `a`, `s`, `b`, `e`, `c`, `f`, `hi`, `d`, `p`, `j`, `r`, `ac`, `an`, `in`, `tx`, `bm`, `sl`, `bs`
- Provide aliases for better UX
- Token from `-t` flag or `TINKOFF_TOKEN_V2` environment variable

## Performance Considerations

- Use `mimalloc` allocator on Linux (release builds)
- LTO enabled for release builds
- Limit concurrent requests to avoid API rate limits (default: 10)
- Use `comfy-table` for efficient terminal output

## Security

- Never commit API tokens
- Use environment variables for sensitive data
- Token passed via CLI should not be logged

## Things to do
- Create tests for a new functionality
- Write tests in AAA pattern
- Code must be pass all clippy pedantic validations
- Result code must be formatted using cargo fmt
- Write code comments only in English

## Things NOT to do
- Dont use unsafe code
- Dont use unwrap or expect to get Option or Result
- Dont search performmance, copy/paste, architecture problems in tests
- Dont suppress clippy warnings using procedure macro like #[allow(clippy::unused_self)]
- Dont create new macros
- Dont write trivial code comments
