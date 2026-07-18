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
- Use `TIResult<T>` for Tinkoff API specific operations
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

```rust
async fn with_retry<T, F, Fut>(f: F) -> Result<T>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<T, TIError>>,
{
    let mut delay = Duration::from_millis(100);
    for attempt in 1..=5 {
        match f().await {
            Ok(v) => return Ok(v),
            Err(e) if attempt == 5 => return Err(eyre::eyre!("{e:?}")),
            Err(_) => {
                sleep(delay).await;
                delay *= 2;
            }
        }
    }
    unreachable!()
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

#### Macro Usage

The project uses macros for reducing boilerplate:

- `collect!` - Transform API responses into HashMap
- `impl_get_until_done!` - Generate methods with retry logic
- `impl_get_instrument_method!` - Generate instrument fetching methods

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
├── main.rs      # CLI entry point, command handling
├── lib.rs       # Library exports, utility conversions
├── client.rs    # Tinkoff API client implementation
├── domain.rs    # Core types, business logic, Display implementations
├── progress.rs  # Progress indicators and UI
└── ux.rs        # User experience utilities, table formatting
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

- Methods ending with `_until_done` include retry logic
- Methods without suffix are raw API calls
- Use `Arc<T>` for sharing client instances across tasks

## CLI Guidelines

- Use `clap` for argument parsing
- Short commands: `a`, `s`, `b`, `e`, `c`, `f`, `hi`, `d`, `p`
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
- Dont write trivial code comments
