use std::{collections::HashMap, env, future::Future, pin::Pin, str::FromStr};

use clap::{Arg, ArgAction, ArgMatches, Command, command};
use color_eyre::eyre::{self, Context, Result};
use std::sync::Arc;
use tokio::task::JoinSet;

use itertools::Itertools;
use t_invest_sdk::api::{AccountType, InstrumentShort, PortfolioPosition};
use tinkoff::{
    account_status_name, account_type_name,
    client::{AccountSelector, TinkoffInvestment},
    domain::{
        Instrument,
        calendar::CalendarKind,
        risk::{TARGET_ASSET_TYPES, TargetAllocation},
    },
    parse_account_type,
    progress::Progresser,
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

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;
    ux::clear_screen();
    let cli = build_cli().get_matches();

    let config = AppConfig::from_matches(&cli)?;

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
        ALL_CMD => Box::pin(all(config, !matches.get_flag("aggregate"))),
        SHARES_CMD => Box::pin(asset(config, "share")),
        BONDS_CMD => Box::pin(asset(config, "bond")),
        ETFS_CMD => Box::pin(asset(config, "etf")),
        CURR_CMD => Box::pin(asset(config, "currency")),
        FUTURES_CMD => Box::pin(asset(config, "futures")),
        HISTORY_CMD => Box::pin(history(config, matches)),
        DIVIDENDS_CMD => Box::pin(calendar(config, matches, CalendarKind::Dividends)),
        COUPONS_CMD => Box::pin(calendar(config, matches, CalendarKind::Coupons)),
        COMBINED_CMD => Box::pin(calendar(config, matches, CalendarKind::Combined)),
        RISK_CMD => Box::pin(risk(config, matches)),
        ACCOUNTS_CMD => Box::pin(accounts(config)),
        ANALYTICS_CMD => Box::pin(analytics(config)),
        _ => Box::pin(async { Ok(()) }),
    }
}

/// Prints portfolio positions of the given API instrument type (`share`, `bond`, etc.).
async fn asset(config: &AppConfig, instrument_type: &str) -> Result<()> {
    let client = TinkoffInvestment::new(config.token.clone());
    let portfolio = client.get_portfolio_until_done(&config.account).await?;

    let positions = portfolio
        .positions
        .into_iter()
        .filter(|p| p.instrument_type == instrument_type)
        .collect_vec();
    let instruments = client.get_instruments_for_positions(&positions).await;

    print_positions(
        &client,
        Arc::new(instruments),
        &positions,
        &portfolio.account_id,
        true,
    )
    .await;
    Ok(())
}

async fn all(config: &AppConfig, output_papers: bool) -> Result<()> {
    let client = TinkoffInvestment::new(config.token.clone());
    let (portfolio, instruments) = client
        .get_portfolio_and_instruments(&config.account)
        .await?;

    print_positions(
        &client,
        Arc::new(instruments),
        &portfolio.positions,
        &portfolio.account_id,
        output_papers,
    )
    .await;
    Ok(())
}

async fn history(config: &AppConfig, cmd: &ArgMatches) -> Result<()> {
    let client = TinkoffInvestment::new(config.token.clone());
    let ticker = cmd
        .get_one::<String>("TICKER")
        .ok_or_else(|| eyre::eyre!("No ticker passed"))?;
    let (account, instruments) = tokio::join!(
        client.get_account(&config.account),
        client.find_instruments_by_ticker_until_done(ticker),
    );
    let account = account?;
    let instruments = instruments?;

    let client = Arc::new(client);
    let account_id = account.id.clone();

    let mut set = JoinSet::new();
    for instr in instruments.into_iter().filter(|i| i.ticker.eq(ticker)) {
        let client = Arc::clone(&client);
        let account_id = account_id.clone();
        set.spawn(async move {
            let ops = client
                .get_operations_until_done(account_id, instr.figi.clone())
                .await;
            (instr, ops)
        });
    }

    let mut instruments_with_ops: HashMap<String, InstrumentShort> = HashMap::new();
    let mut operations = vec![];
    while let Some(res) = set.join_next().await {
        match res {
            Ok((instr, Ok(ops))) if !ops.is_empty() => {
                operations.extend(ops);
                instruments_with_ops.insert(instr.figi.clone(), instr);
            }
            Ok((_, Err(e))) => eprintln!("Failed to load operations: {e:?}"),
            Err(e) => eprintln!("Task panicked: {e}"),
            _ => {}
        }
    }

    let Some((_, instrument)) = instruments_with_ops
        .iter()
        .sorted_by(|(a, _), (b, _)| {
            if a.starts_with("TCS") {
                std::cmp::Ordering::Greater
            } else {
                Ord::cmp(a, b)
            }
        })
        .next()
    else {
        return Ok(());
    };

    if let Some(history) = client.history_in_rub(&operations, instrument).await? {
        println!("{history}");
    }
    Ok(())
}

async fn calendar(config: &AppConfig, cmd: &ArgMatches, kind: CalendarKind) -> Result<()> {
    let days = cmd
        .get_one::<u32>("days")
        .copied()
        .ok_or_else(|| eyre::eyre!("Calendar horizon is not set"))?;
    let client = TinkoffInvestment::new(config.token.clone());
    let (portfolio, instruments) = client
        .get_portfolio_and_instruments(&config.account)
        .await?;
    let (calendar, failures) = client
        .get_calendar(&portfolio, &instruments, kind, days)
        .await?;
    println!("{calendar}");
    report_failures(&failures);
    Ok(())
}

/// Prints analyst forecasts and fundamentals of the portfolio shares.
async fn analytics(config: &AppConfig) -> Result<()> {
    let client = TinkoffInvestment::new(config.token.clone());
    let portfolio = client.get_portfolio_until_done(&config.account).await?;
    let shares = portfolio
        .positions
        .into_iter()
        .filter(|p| p.instrument_type == "share")
        .collect_vec();
    let instruments = client.get_instruments_for_positions(&shares).await;
    let (analytics, failures) = client.get_share_analytics(&shares, &instruments).await;
    print!("{analytics}");
    report_failures(&failures);
    Ok(())
}

async fn accounts(config: &AppConfig) -> Result<()> {
    let client = TinkoffInvestment::new(config.token.clone());
    let accounts = client.get_accounts().await?;

    let mut table = ux::new_table();
    table.set_header(["ID", "Name", "Type", "Status", "Opened"]);
    for account in &accounts {
        let opened = account
            .opened_date
            .as_ref()
            .map(|d| {
                tinkoff::to_datetime_utc(Some(d))
                    .format("%Y-%m-%d")
                    .to_string()
            })
            .unwrap_or_default();
        table.add_row([
            account.id.clone(),
            account.name.clone(),
            account_type_name(account.r#type()).to_string(),
            account_status_name(account.status()).to_string(),
            opened,
        ]);
    }
    println!("{table}");
    Ok(())
}

async fn risk(config: &AppConfig, cmd: &ArgMatches) -> Result<()> {
    use tinkoff::domain::risk::{RebalancingAnalysis, RiskAnalysis};

    let client = TinkoffInvestment::new(config.token.clone());
    let (portfolio_data, instruments) = client
        .get_portfolio_and_instruments(&config.account)
        .await?;

    let positions = &portfolio_data.positions;
    let account_id = &portfolio_data.account_id;

    let progress = Arc::new(tinkoff::progress::Progresser::new(positions.len() as u64));
    let (container, failures) = client
        .build_portfolio(
            Arc::new(instruments),
            positions,
            account_id,
            false,
            Some(progress),
        )
        .await;

    let risk_analysis = RiskAnalysis::analyze(&container);
    println!("{risk_analysis}");

    if let Some(target) = cmd.get_one::<TargetAllocation>("target") {
        let rebalancing = RebalancingAnalysis::analyze(&risk_analysis.asset_allocation, target);
        println!("{rebalancing}");
    }
    report_failures(&failures);

    Ok(())
}

async fn print_positions(
    client: &TinkoffInvestment,
    instruments: Arc<HashMap<String, Instrument>>,
    positions: &[PortfolioPosition],
    account_id: &str,
    output_papers: bool,
) {
    let progress = Arc::new(Progresser::new(positions.len() as u64));
    let (container, failures) = client
        .build_portfolio(
            instruments,
            positions,
            account_id,
            output_papers,
            Some(progress),
        )
        .await;
    print!("{container}");
    report_failures(&failures);
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
                .value_parser(parse_account_type)
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
