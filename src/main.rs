use std::{collections::HashMap, env, future::Future, pin::Pin, str::FromStr};

use clap::{ArgAction, ArgMatches, Command, command};
use color_eyre::eyre::{self, Context, Result};
use std::sync::Arc;
use tokio::task::JoinSet;

use itertools::Itertools;
use tinkoff::{
    client::TinkoffInvestment,
    domain::{
        Instrument,
        calendar::CalendarKind,
        risk::{TARGET_ASSET_TYPES, TargetAllocation},
    },
    parse_account_type,
    progress::Progresser,
    ux,
};
use tinkoff_invest_api::tcs::{AccountType, InstrumentShort, PortfolioPosition};

struct AppConfig {
    token: String,
    account: AccountType,
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

        let account = matches
            .get_one::<AccountType>("account")
            .copied()
            .expect("account has a default value");

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
        DIVIDENDS_CMD => Box::pin(calendar(config, CalendarKind::Dividends)),
        COUPONS_CMD => Box::pin(calendar(config, CalendarKind::Coupons)),
        COMBINED_CMD => Box::pin(calendar(config, CalendarKind::Combined)),
        RISK_CMD => Box::pin(risk(config, matches)),
        _ => Box::pin(async { Ok(()) }),
    }
}

/// Prints portfolio positions of the given API instrument type (`share`, `bond`, etc.).
async fn asset(config: &AppConfig, instrument_type: &str) -> Result<()> {
    let client = TinkoffInvestment::new(config.token.clone());
    let portfolio = client.get_portfolio_until_done(config.account).await?;

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
    let (portfolio, instruments) = client.get_portfolio_and_instruments(config.account).await?;

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
        client.get_account(config.account),
        client.find_instruments_by_ticker(ticker.clone()),
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

async fn calendar(config: &AppConfig, kind: CalendarKind) -> Result<()> {
    let client = TinkoffInvestment::new(config.token.clone());
    let (portfolio, instruments) = client.get_portfolio_and_instruments(config.account).await?;
    let (calendar, failures) = client.get_calendar(&portfolio, &instruments, kind).await?;
    println!("{calendar}");
    report_failures(&failures);
    Ok(())
}

async fn risk(config: &AppConfig, cmd: &ArgMatches) -> Result<()> {
    use tinkoff::domain::risk::{RebalancingAnalysis, RiskAnalysis};

    let client = TinkoffInvestment::new(config.token.clone());
    let (portfolio_data, instruments) =
        client.get_portfolio_and_instruments(config.account).await?;

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
                .help("Account type: tinkoff (broker, default), iis, invest-box, invest-fund"),
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

fn dividends_cmd() -> Command {
    Command::new(DIVIDENDS_CMD)
        .aliases(["dividends"])
        .about("Get dividend calendar for portfolio")
}

fn coupons_cmd() -> Command {
    Command::new(COUPONS_CMD)
        .aliases(["coupons"])
        .about("Get coupon calendar for portfolio bonds")
}

fn combined_cmd() -> Command {
    Command::new(COMBINED_CMD)
        .aliases(["combined", "join"])
        .about("Get combined dividend and coupon calendar")
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
