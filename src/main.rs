use std::{collections::HashMap, env};

use clap::{ArgAction, ArgMatches, Command, command};
use color_eyre::eyre::{self, Context, Result};
use std::sync::Arc;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use itertools::Itertools;
use tinkoff::{
    client::TinkoffInvestment,
    domain::{
        Asset, CouponProfit, DividentProfit, History, Instrument, NoneProfit, Paper, Portfolio,
        Profit,
    },
    progress::{Progress, Progresser},
    ux,
};
use tinkoff_invest_api::tcs::{AccountType, InstrumentShort, PortfolioPosition};

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

const MAX_CONCURRENT_REQUESTS: usize = 10;

enum AssetPaper {
    Bond(Paper<CouponProfit>),
    Share(Paper<DividentProfit>),
    Etf(Paper<NoneProfit>),
    Currency(Paper<NoneProfit>),
    Future(Paper<NoneProfit>),
}

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;
    ux::clear_screen();
    let cli = build_cli().get_matches();

    let token = if let Some(t) = cli.get_one::<String>("token") {
        t.clone()
    } else {
        env::var("TINKOFF_TOKEN_V2").wrap_err_with(|| {
            "API token required either from -t option or from TINKOFF_TOKEN_V2 environment variable"
        })?
    };

    match cli.subcommand() {
        Some((ALL_CMD, cmd)) => Box::pin(all(token, !cmd.get_flag("aggregate"))).await?,
        Some((SHARES_CMD, _)) => asset(token, AssetType::Shares).await?,
        Some((BONDS_CMD, _)) => asset(token, AssetType::Bonds).await?,
        Some((ETFS_CMD, _)) => asset(token, AssetType::Etfs).await?,
        Some((CURR_CMD, _)) => asset(token, AssetType::Currencies).await?,
        Some((FUTURES_CMD, _)) => asset(token, AssetType::Futures).await?,
        Some((HISTORY_CMD, cmd)) => history(token, cmd).await?,
        Some((DIVIDENDS_CMD, _)) => dividends(token).await?,
        _ => {}
    }
    Ok(())
}

enum AssetType {
    Bonds,
    Shares,
    Etfs,
    Futures,
    Currencies,
}

impl AssetType {
    fn instrument_type(&self) -> &'static str {
        match self {
            AssetType::Bonds => "bond",
            AssetType::Shares => "share",
            AssetType::Etfs => "etf",
            AssetType::Futures => "futures",
            AssetType::Currencies => "currency",
        }
    }

    async fn fetch_instruments(
        &self,
        client: &TinkoffInvestment,
    ) -> Result<HashMap<String, Instrument>> {
        match self {
            AssetType::Bonds => client.get_all_bonds_until_done().await,
            AssetType::Shares => client.get_all_shares_until_done().await,
            AssetType::Etfs => client.get_all_etfs_until_done().await,
            AssetType::Futures => client.get_all_futures_until_done().await,
            AssetType::Currencies => client.get_all_currencies_until_done().await,
        }
    }
}

async fn asset(token: String, asset_type: AssetType) -> Result<()> {
    let client = TinkoffInvestment::new(token);
    let instruments = asset_type.fetch_instruments(&client).await?;
    let portfolio = client
        .get_portfolio_until_done(AccountType::Tinkoff)
        .await?;

    let positions = portfolio
        .positions
        .into_iter()
        .filter(|p| p.instrument_type == asset_type.instrument_type())
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

async fn all(token: String, output_papers: bool) -> Result<()> {
    let client = TinkoffInvestment::new(token);

    let (all, shares, etfs, currencies, futures, portfolio) = tokio::join!(
        client.get_all_bonds_until_done(),
        client.get_all_shares_until_done(),
        client.get_all_etfs_until_done(),
        client.get_all_currencies_until_done(),
        client.get_all_futures_until_done(),
        client.get_portfolio_until_done(AccountType::Tinkoff),
    );
    let mut all = all?;
    let shares = shares?;
    let etfs = etfs?;
    let currencies = currencies?;
    let futures = futures?;

    all.extend(shares);
    all.extend(etfs);
    all.extend(currencies);
    all.extend(futures);

    let portfolio = portfolio?;
    print_positions(
        &client,
        &all,
        &portfolio.positions,
        &portfolio.account_id,
        output_papers,
    )
    .await;
    Ok(())
}

async fn history(token: String, cmd: &ArgMatches) -> Result<()> {
    let client = TinkoffInvestment::new(token);
    let ticker = cmd
        .get_one::<String>("TICKER")
        .ok_or_else(|| eyre::eyre!("No ticker passed"))?;
    let (account, instruments) = tokio::join!(
        client.get_account(AccountType::Tinkoff),
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

async fn print_positions(
    client: &TinkoffInvestment,
    instruments: &HashMap<String, Instrument>,
    positions: &[PortfolioPosition],
    account_id: &str,
    output_papers: bool,
) {
    fn add_paper_into_container<P: Profit>(asset: &mut Asset<P>, paper: Option<Paper<P>>) {
        if let Some(p) = paper {
            asset.add_paper(p);
        }
    }

    let mut container = Portfolio::new(output_papers);

    // To avoid borrow errors
    let client = Arc::new(client.clone());
    let instruments = Arc::new(instruments.clone());
    let account_id = account_id.to_string();

    let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_REQUESTS));
    let progresser = Arc::new(Progresser::new(positions.len() as u64));

    let mut set = JoinSet::new();

    for p in positions {
        let client = Arc::clone(&client);
        let instruments = Arc::clone(&instruments);
        let account_id = account_id.clone();
        let permit = semaphore.clone().acquire_owned().await.unwrap();
        let progresser = Arc::clone(&progresser);
        let p = p.clone();

        set.spawn(async move {
            let _permit = permit;

            let paper = match p.instrument_type.as_str() {
                "bond" => client
                    .create_paper_from_position(&instruments, account_id, &p, CouponProfit)
                    .await
                    .map(AssetPaper::Bond),
                "share" => client
                    .create_paper_from_position(&instruments, account_id, &p, DividentProfit)
                    .await
                    .map(AssetPaper::Share),
                "etf" => client
                    .create_paper_from_position(&instruments, account_id, &p, NoneProfit)
                    .await
                    .map(AssetPaper::Etf),
                "currency" => client
                    .create_paper_from_position(&instruments, account_id, &p, NoneProfit)
                    .await
                    .map(AssetPaper::Currency),
                "futures" => client
                    .create_paper_from_position(&instruments, account_id, &p, NoneProfit)
                    .await
                    .map(AssetPaper::Future),
                _ => None,
            };

            // Atomic progress update
            progresser.progress();

            paper
        });
    }

    // Collect results
    while let Some(res) = set.join_next().await {
        match res {
            Ok(Some(AssetPaper::Bond(p))) => {
                add_paper_into_container(&mut container.bonds, Some(p));
            }
            Ok(Some(AssetPaper::Share(p))) => {
                add_paper_into_container(&mut container.shares, Some(p));
            }
            Ok(Some(AssetPaper::Etf(p))) => add_paper_into_container(&mut container.etfs, Some(p)),
            Ok(Some(AssetPaper::Currency(p))) => {
                add_paper_into_container(&mut container.currencies, Some(p));
            }
            Ok(Some(AssetPaper::Future(p))) => {
                add_paper_into_container(&mut container.futures, Some(p));
            }
            Ok(None) => {}
            Err(e) => eprintln!("Task panicked or cancelled: {e}"),
        }
    }

    progresser.finish();
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
        .subcommand(all_cmd())
        .subcommand(shares_cmd())
        .subcommand(bonds_cmd())
        .subcommand(etfs_cmd())
        .subcommand(currencies_cmd())
        .subcommand(futures_cmd())
        .subcommand(history_cmd())
        .subcommand(dividends_cmd())
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

async fn dividends(token: String) -> Result<()> {
    let client = TinkoffInvestment::new(token);

    let portfolio = client
        .get_portfolio_until_done(AccountType::Tinkoff)
        .await?;

    // Get all instruments for portfolio positions
    let mut all_instruments = HashMap::new();

    // Fetch instruments for all positions
    let (shares, bonds, etfs, currencies, futures) = tokio::join!(
        client.get_all_shares_until_done(),
        client.get_all_bonds_until_done(),
        client.get_all_etfs_until_done(),
        client.get_all_currencies_until_done(),
        client.get_all_futures_until_done(),
    );

    let iter = [shares, bonds, etfs, currencies, futures].into_iter();
    for instrs in iter.flatten() {
        all_instruments.extend(instrs);
    }

    let calendar = client
        .get_dividend_calendar(portfolio.account_id, &portfolio.positions, &all_instruments)
        .await?;

    println!("{calendar}");
    Ok(())
}
