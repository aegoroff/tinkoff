use std::{collections::HashMap, env, future::Future, pin::Pin};

use clap::{ArgAction, ArgMatches, Command, command};
use color_eyre::eyre::{self, Context, Result};
use std::sync::Arc;
use tokio::task::JoinSet;

use itertools::Itertools;
use tinkoff::{
    client::{AccountPortfolio, InstrumentCatalog, TinkoffInvestment},
    domain::{History, Instrument},
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
        SHARES_CMD => Box::pin(asset(config, InstrumentCatalog::Shares)),
        BONDS_CMD => Box::pin(asset(config, InstrumentCatalog::Bonds)),
        ETFS_CMD => Box::pin(asset(config, InstrumentCatalog::Etfs)),
        CURR_CMD => Box::pin(asset(config, InstrumentCatalog::Currencies)),
        FUTURES_CMD => Box::pin(asset(config, InstrumentCatalog::Futures)),
        HISTORY_CMD => Box::pin(history(config, matches)),
        DIVIDENDS_CMD => Box::pin(dividends(config)),
        COUPONS_CMD => Box::pin(coupons(config)),
        _ => Box::pin(async { Ok(()) }),
    }
}

async fn asset(config: &AppConfig, catalog: InstrumentCatalog) -> Result<()> {
    let client = TinkoffInvestment::new(config.token.clone());
    let (portfolio, instruments) = client
        .get_portfolio_and_catalog(config.account, catalog)
        .await?;

    let positions = portfolio
        .positions
        .into_iter()
        .filter(|p| p.instrument_type == catalog.instrument_type())
        .collect_vec();

    print_positions(
        &client,
        &instruments,
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
        &instruments,
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

    if let Some(history) = History::new(&operations, instrument) {
        println!("{history}");
    }
    Ok(())
}

async fn dividends(config: &AppConfig) -> Result<()> {
    let (client, portfolio, instruments) = Box::pin(portfolio_with_instruments(config)).await?;
    let calendar = client
        .get_dividend_calendar(&portfolio, &instruments)
        .await?;
    println!("{calendar}");
    Ok(())
}

async fn coupons(config: &AppConfig) -> Result<()> {
    let (client, portfolio, instruments) = Box::pin(portfolio_with_instruments(config)).await?;
    let calendar = client.get_coupon_calendar(&portfolio, &instruments).await?;
    println!("{calendar}");
    Ok(())
}

async fn print_positions(
    client: &TinkoffInvestment,
    instruments: &HashMap<String, Instrument>,
    positions: &[PortfolioPosition],
    account_id: &str,
    output_papers: bool,
) {
    let progress = Arc::new(Progresser::new(positions.len() as u64));
    let container = client
        .build_portfolio(
            instruments,
            positions,
            account_id,
            output_papers,
            Some(progress),
        )
        .await;
    print!("{container}");
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

async fn portfolio_with_instruments(
    config: &AppConfig,
) -> Result<(
    TinkoffInvestment,
    AccountPortfolio,
    HashMap<String, Instrument>,
)> {
    let client = TinkoffInvestment::new(config.token.clone());
    let (portfolio, instruments) = client.get_portfolio_and_instruments(config.account).await?;
    Ok((client, portfolio, instruments))
}
