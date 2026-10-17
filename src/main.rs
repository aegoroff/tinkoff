use std::{env, future::Future, pin::Pin, str::FromStr};

use clap::{Arg, ArgAction, ArgMatches, Command, command};
use color_eyre::eyre::{self, Context, Result};
use std::sync::Arc;

use tinkoff::{
    client::TinkoffInvestment,
    domain::{
        InstrumentKind, Portfolio,
        account::AccountType,
        calendar::CalendarKind,
        risk::{TARGET_ASSET_TYPES, TargetAllocation},
    },
    progress::Progresser,
    reports::{AccountSelector, Reports},
    ux,
};

struct AppConfig {
    token: String,
    account: AccountSelector,
}

impl AppConfig {
    fn from_matches(matches: &ArgMatches) -> Result<Self> {
        let token = if let Some(t) = matches.get_one::<String>("token") {
            t.clone()
        } else {
            env::var("TINKOFF_TOKEN_V2").wrap_err_with(|| {
                "API token required either from -t option or from TINKOFF_TOKEN_V2 environment variable"
            })?
        };

        let account = if let Some(id) = matches.get_one::<String>("account-id") {
            AccountSelector::Id(id.clone())
        } else {
            let account_type = matches
                .get_one::<AccountType>("account")
                .copied()
                .ok_or_else(|| eyre::eyre!("Account type is not set"))?;
            AccountSelector::Type(account_type)
        };

        Ok(Self { token, account })
    }

    fn reports(&self) -> Reports<TinkoffInvestment> {
        Reports::new(TinkoffInvestment::new(self.token.clone()))
    }
}

#[cfg(target_os = "linux")]
use mimalloc::MiMalloc;

#[cfg(target_os = "linux")]
#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

#[macro_use]
extern crate clap;

const ALL_CMD: &str = "a";
const SHARES_CMD: &str = "s";
const BONDS_CMD: &str = "b";
const ETFS_CMD: &str = "e";
const CURR_CMD: &str = "c";
const FUTURES_CMD: &str = "f";
const HISTORY_CMD: &str = "hi";
const DIVIDENDS_CMD: &str = "d";
const COUPONS_CMD: &str = "p";
const COMBINED_CMD: &str = "j";
const RISK_CMD: &str = "r";
const ACCOUNTS_CMD: &str = "ac";
const ANALYTICS_CMD: &str = "an";
const INCOME_CMD: &str = "in";
const TAXES_CMD: &str = "tx";
const BENCHMARK_CMD: &str = "bm";

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;
    // Cleared once arguments are valid, so help and usage errors stay on the screen.
    let cli = build_cli().get_matches();
    let config = AppConfig::from_matches(&cli)?;
    ux::clear_screen();

    if let Some(sub) = cli.subcommand() {
        run_subcommand(&config, sub).await?;
    }
    Ok(())
}

fn run_subcommand<'a>(
    config: &'a AppConfig,
    (name, matches): (&'a str, &'a ArgMatches),
) -> Pin<Box<dyn Future<Output = Result<()>> + 'a>> {
    match name {
        ALL_CMD => Box::pin(positions(config, None, !matches.get_flag("aggregate"))),
        SHARES_CMD => Box::pin(positions(config, Some(InstrumentKind::Share), true)),
        BONDS_CMD => Box::pin(positions(config, Some(InstrumentKind::Bond), true)),
        ETFS_CMD => Box::pin(positions(config, Some(InstrumentKind::Etf), true)),
        CURR_CMD => Box::pin(positions(config, Some(InstrumentKind::Currency), true)),
        FUTURES_CMD => Box::pin(positions(config, Some(InstrumentKind::Future), true)),
        HISTORY_CMD => Box::pin(history(config, matches)),
        DIVIDENDS_CMD => Box::pin(calendar(config, matches, CalendarKind::Dividends)),
        COUPONS_CMD => Box::pin(calendar(config, matches, CalendarKind::Coupons)),
        COMBINED_CMD => Box::pin(calendar(config, matches, CalendarKind::Combined)),
        RISK_CMD => Box::pin(risk(config, matches)),
        ACCOUNTS_CMD => Box::pin(accounts(config)),
        ANALYTICS_CMD => Box::pin(analytics(config, matches)),
        INCOME_CMD => Box::pin(income(config)),
        TAXES_CMD => Box::pin(taxes(config)),
        BENCHMARK_CMD => Box::pin(benchmark(config)),
        _ => Box::pin(async { Ok(()) }),
    }
}

/// Prints portfolio positions of the given kind or all of them,
/// with paper cards when `output_papers` is set.
async fn positions(
    config: &AppConfig,
    kind: Option<InstrumentKind>,
    output_papers: bool,
) -> Result<()> {
    let (container, failures) = load_papers(config, &config.reports(), kind, output_papers).await?;
    print!("{container}");
    report_failures(&failures);
    Ok(())
}

async fn history(config: &AppConfig, cmd: &ArgMatches) -> Result<()> {
    let ticker = cmd
        .get_one::<String>("TICKER")
        .ok_or_else(|| eyre::eyre!("No ticker passed"))?;
    let history = config
        .reports()
        .get_history(&config.account, ticker)
        .await?;
    println!("{history}");
    Ok(())
}

async fn calendar(config: &AppConfig, cmd: &ArgMatches, kind: CalendarKind) -> Result<()> {
    let days = cmd
        .get_one::<u32>("days")
        .copied()
        .ok_or_else(|| eyre::eyre!("Calendar horizon is not set"))?;
    let reports = config.reports();
    let portfolio = reports.load_portfolio(&config.account, None).await?;
    let (calendar, failures) = reports.get_calendar(&portfolio, kind, days).await?;
    println!("{calendar}");
    report_failures(&failures);
    Ok(())
}

/// Prints passive income expected within a year and the portfolio yield.
async fn income(config: &AppConfig) -> Result<()> {
    let reports = config.reports();
    let portfolio = reports.load_portfolio(&config.account, None).await?;
    let (forecast, failures) = reports.get_income_forecast(&portfolio).await?;
    println!("{forecast}");
    report_failures(&failures);
    Ok(())
}

/// Prints returns of the portfolio and its asset types compared with market indices.
async fn benchmark(config: &AppConfig) -> Result<()> {
    use chrono::{Datelike, Utc};
    use tinkoff::domain::benchmark::BenchmarkComparison;

    let reports = config.reports();
    let (container, mut failures) = load_papers(config, &reports, None, false).await?;

    let now = Utc::now();
    let since_year = container
        .payments()
        .iter()
        .map(|f| f.date.year())
        .min()
        .unwrap_or_else(|| now.year());
    let (indices, index_failures) = reports.get_index_histories(since_year).await?;
    failures.extend(index_failures);

    println!("{}", BenchmarkComparison::new(&container, indices, now));
    report_failures(&failures);
    Ok(())
}

/// Prints income and taxes withheld by year.
async fn taxes(config: &AppConfig) -> Result<()> {
    let reports = config.reports();
    let report = reports.get_tax_report(&config.account).await?;
    println!("{report}");
    Ok(())
}

/// Prints analyst forecasts and fundamentals of the portfolio shares,
/// or detailed ones of the share with the given ticker only.
async fn analytics(config: &AppConfig, cmd: &ArgMatches) -> Result<()> {
    let reports = config.reports();
    let mut shares = reports
        .load_portfolio(&config.account, Some(InstrumentKind::Share))
        .await?;

    let Some(ticker) = cmd.get_one::<String>("TICKER") else {
        let (analytics, failures) = reports.get_share_analytics(&shares).await;
        print!("{analytics}");
        report_failures(&failures);
        return Ok(());
    };

    shares
        .holdings
        .retain(|h| h.instrument.ticker.as_str().eq_ignore_ascii_case(ticker));
    if shares.holdings.is_empty() {
        return Err(eyre::eyre!("No share {ticker} in the portfolio"));
    }
    let (analytics, failures) = reports.get_share_analytics(&shares).await;
    for share in analytics.shares() {
        println!("{share}");
    }
    report_failures(&failures);
    Ok(())
}

async fn accounts(config: &AppConfig) -> Result<()> {
    let reports = config.reports();
    let accounts = reports.get_accounts().await?;

    let mut table = ux::new_table();
    table.set_header(["ID", "Name", "Type", "Status", "Opened"]);
    for account in &accounts {
        let opened = account
            .opened
            .map(|d| d.format("%Y-%m-%d").to_string())
            .unwrap_or_default();
        table.add_row([
            account.id.clone(),
            account.name.clone(),
            account.account_type.name().to_string(),
            account.status.name().to_string(),
            opened,
        ]);
    }
    println!("{table}");
    Ok(())
}

async fn risk(config: &AppConfig, cmd: &ArgMatches) -> Result<()> {
    use tinkoff::domain::risk::{RebalancingAnalysis, RiskAnalysis};

    let reports = config.reports();
    let (container, failures) = load_papers(config, &reports, None, false).await?;

    let risk_analysis = RiskAnalysis::analyze(&container);
    println!("{risk_analysis}");

    if let Some(target) = cmd.get_one::<TargetAllocation>("target") {
        let rebalancing = RebalancingAnalysis::analyze(&risk_analysis.asset_allocation, target);
        println!("{rebalancing}");
    }
    report_failures(&failures);

    Ok(())
}

/// Loads papers of the account portfolio, of the given kind or all, showing the progress;
/// positions that failed to load are returned alongside.
async fn load_papers(
    config: &AppConfig,
    reports: &Reports<TinkoffInvestment>,
    kind: Option<InstrumentKind>,
    output_papers: bool,
) -> Result<(Portfolio, Vec<eyre::Report>)> {
    let portfolio = reports.load_portfolio(&config.account, kind).await?;
    let progress = Arc::new(Progresser::new(portfolio.holdings.len() as u64));
    Ok(reports
        .build_portfolio(&portfolio, output_papers, Some(progress))
        .await)
}

/// Warns that the output is incomplete because some positions failed to load.
fn report_failures(failures: &[eyre::Report]) {
    if failures.is_empty() {
        return;
    }
    eprintln!(
        "Warning: {} position(s) failed to load, output above is incomplete:",
        failures.len()
    );
    for e in failures {
        eprintln!("  - {e:#}");
    }
}

fn build_cli() -> Command {
    #![allow(non_upper_case_globals)]
    command!(crate_name!())
        .arg_required_else_help(true)
        .version(crate_version!())
        .author(crate_authors!("\n"))
        .about(crate_description!())
        .arg(arg!(-t --token <VALUE>).required(false).help(
            "Tinkoff API v2 token. If not set TINKOFF_TOKEN_V2 environment variable will be used",
        ))
        .arg(
            arg!(--account <TYPE>)
                .required(false)
                .default_value("tinkoff")
                .value_parser(AccountType::parse)
                .help(
                    "Account type: tinkoff (broker, default), iis, invest-box, invest-fund, \
                     debit, saving, dfa. Selects the only open account of this type",
                ),
        )
        .arg(
            arg!(--"account-id" <ID>)
                .required(false)
                .help("Account ID (see the ac command); takes precedence over --account"),
        )
        .subcommand(all_cmd())
        .subcommand(shares_cmd())
        .subcommand(bonds_cmd())
        .subcommand(etfs_cmd())
        .subcommand(currencies_cmd())
        .subcommand(futures_cmd())
        .subcommand(history_cmd())
        .subcommand(dividends_cmd())
        .subcommand(coupons_cmd())
        .subcommand(combined_cmd())
        .subcommand(risk_cmd())
        .subcommand(accounts_cmd())
        .subcommand(analytics_cmd())
        .subcommand(income_cmd())
        .subcommand(taxes_cmd())
        .subcommand(benchmark_cmd())
}

fn all_cmd() -> Command {
    Command::new(ALL_CMD)
        .aliases(["all"])
        .about("Get all portfolio")
        .arg(
            arg!(-a - -aggregate)
                .required(false)
                .action(ArgAction::SetTrue)
                .help("Output only aggregated information about assets"),
        )
}

fn shares_cmd() -> Command {
    Command::new(SHARES_CMD)
        .aliases(["shares"])
        .about("Get portfolio shares")
}

fn bonds_cmd() -> Command {
    Command::new(BONDS_CMD)
        .aliases(["bonds"])
        .about("Get portfolio bonds")
}

fn etfs_cmd() -> Command {
    Command::new(ETFS_CMD)
        .aliases(["etfs"])
        .about("Get portfolio etfs")
}

fn currencies_cmd() -> Command {
    Command::new(CURR_CMD)
        .aliases(["currencies"])
        .about("Get portfolio currencies")
}

fn futures_cmd() -> Command {
    Command::new(FUTURES_CMD)
        .aliases(["futures"])
        .about("Get portfolio futures")
}

fn history_cmd() -> Command {
    Command::new(HISTORY_CMD)
        .aliases(["history"])
        .about("Get an instrument history")
        .arg(arg!([TICKER]).help("Instrument's tiker").required(true))
}

/// Calendar horizon in days.
fn days_arg() -> Arg {
    arg!(--days <N>)
        .required(false)
        .default_value("365")
        .value_parser(value_parser!(u32).range(1..=36_500))
        .help("Show payments due within N days from today")
}

fn dividends_cmd() -> Command {
    Command::new(DIVIDENDS_CMD)
        .aliases(["dividends"])
        .about("Get dividend calendar for portfolio")
        .arg(days_arg())
}

fn coupons_cmd() -> Command {
    Command::new(COUPONS_CMD)
        .aliases(["coupons"])
        .about("Get bond payments calendar: coupons, amortizations and maturities")
        .arg(days_arg())
}

fn combined_cmd() -> Command {
    Command::new(COMBINED_CMD)
        .aliases(["combined", "join"])
        .about("Get combined dividend and bond payments calendar")
        .arg(days_arg())
}

fn accounts_cmd() -> Command {
    Command::new(ACCOUNTS_CMD)
        .aliases(["accounts"])
        .about("List accounts")
}

fn analytics_cmd() -> Command {
    Command::new(ANALYTICS_CMD)
        .aliases(["analytics", "forecasts", "fundamentals"])
        .about("Get analyst forecasts and fundamentals of portfolio shares")
        .arg(
            arg!([TICKER])
                .help("Ticker of a portfolio share to get detailed analytics of it only")
                .required(false),
        )
}

fn income_cmd() -> Command {
    Command::new(INCOME_CMD)
        .aliases(["income", "passive"])
        .about("Forecast passive income for a year: coupons, dividends and current yield")
}

fn taxes_cmd() -> Command {
    Command::new(TAXES_CMD)
        .aliases(["taxes", "tax"])
        .about("Get income and taxes withheld by year")
}

fn benchmark_cmd() -> Command {
    Command::new(BENCHMARK_CMD)
        .aliases(["benchmark", "compare"])
        .about("Compare returns of the portfolio with IMOEX, MCFTR, RGBI and RGBITR indices")
}

fn risk_cmd() -> Command {
    Command::new(RISK_CMD)
        .aliases(["risk", "risk-analysis"])
        .about("Analyze portfolio risk metrics")
        .arg(
            arg!(--target <ALLOCATION>)
                .required(false)
                .value_parser(TargetAllocation::from_str)
                .help(format!(
                    "Target allocation to get rebalancing recommendations: a preset or \
                     percents, e.g. bonds=60,shares=30,etfs=10. Presets: {}. \
                     Asset types: {TARGET_ASSET_TYPES}; omitted ones are 0, the sum must be 100",
                    TargetAllocation::presets_help()
                )),
        )
}
