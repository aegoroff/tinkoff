use std::{collections::HashMap, env};

use clap::{ArgAction, ArgMatches, Command, command};
use color_eyre::eyre::{Context, Result};

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
        Some((ALL_CMD, cmd)) => Box::pin(all(token, !cmd.get_flag("aggregate"))).await,
        Some((SHARES_CMD, _)) => shares(token).await,
        Some((BONDS_CMD, _)) => bonds(token).await,
        Some((ETFS_CMD, _)) => etfs(token).await,
        Some((CURR_CMD, _)) => currencies(token).await,
        Some((FUTURES_CMD, _)) => futures(token).await,
        Some((HISTORY_CMD, cmd)) => history(token, cmd).await,
        _ => {}
    }
    Ok(())
}

async fn all(token: String, output_papers: bool) {
    let client = TinkoffInvestment::new(token);

    let (mut all, shares, etfs, currencies, futures, portfolio) = tokio::join!(
        client.get_all_bonds_until_done(),
        client.get_all_shares_until_done(),
        client.get_all_etfs_until_done(),
        client.get_all_currencies_until_done(),
        client.get_all_futures_until_done(),
        client.get_portfolio_until_done(AccountType::Tinkoff),
    );

    all.extend(shares);
    all.extend(etfs);
    all.extend(currencies);
    all.extend(futures);

    print_positions(
        &client,
        &all,
        &portfolio.positions,
        &portfolio.account_id,
        output_papers,
    )
    .await;
}

async fn history(token: String, cmd: &ArgMatches) {
    let client = TinkoffInvestment::new(token);
    let Some(ticker) = cmd.get_one::<String>("TICKER") else {
        return;
    };
    let (account, instruments) = tokio::join!(
        client.get_account(AccountType::Tinkoff),
        client.find_instruments_by_ticker(ticker.clone()),
    );
    let Ok(account) = account else {
        return;
    };

    let Ok(instruments) = instruments else {
        return;
    };

    let mut instruments_with_ops: HashMap<&String, &InstrumentShort> = HashMap::new();
    let mut operations = vec![];
    for instr in instruments.iter().filter(|i| i.ticker.eq(ticker)) {
        let instr_operations = client
            .get_operations_until_done(account.id.clone(), instr.figi.clone())
            .await;

        operations.extend(instr_operations.iter().cloned());
        if !instr_operations.is_empty() {
            instruments_with_ops.insert(&instr.figi, instr);
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
        return;
    };

    if let Some(history) = History::new(&operations, instrument) {
        println!("{history}");
    }
}

async fn bonds(token: String) {
    let client = TinkoffInvestment::new(token);
    let instruments = client.get_all_bonds_until_done().await;
    asset(client, instruments, "bond").await;
}

async fn shares(token: String) {
    let client = TinkoffInvestment::new(token);
    let instruments = client.get_all_shares_until_done().await;
    asset(client, instruments, "share").await;
}

async fn etfs(token: String) {
    let client = TinkoffInvestment::new(token);
    let instruments = client.get_all_etfs_until_done().await;
    asset(client, instruments, "etf").await;
}

async fn futures(token: String) {
    let client = TinkoffInvestment::new(token);
    let instruments = client.get_all_futures_until_done().await;
    asset(client, instruments, "futures").await;
}

async fn currencies(token: String) {
    let client = TinkoffInvestment::new(token);
    let instruments = client.get_all_currencies_until_done().await;
    asset(client, instruments, "currency").await;
}

async fn asset(
    client: TinkoffInvestment,
    instruments: HashMap<String, Instrument>,
    instrument_type: &str,
) {
    let portfolio = client.get_portfolio_until_done(AccountType::Tinkoff).await;

    let positions = portfolio
        .positions
        .into_iter()
        .filter(|p| p.instrument_type == instrument_type)
        .collect_vec();

    print_positions(
        &client,
        &instruments,
        &positions,
        &portfolio.account_id,
        true,
    )
    .await;
}

async fn print_positions(
    client: &TinkoffInvestment,
    instruments: &HashMap<String, Instrument>,
    positions: &Vec<PortfolioPosition>,
    account_id: &str,
    output_papers: bool,
) {
    fn add_paper_into_container<P: Profit>(asset: &mut Asset<P>, paper: Option<Paper<P>>) {
        if let Some(p) = paper {
            asset.add_paper(p);
        }
    }
    let mut container = Portfolio::new(output_papers);
    let mut progresser = Progresser::new(positions.len() as u64);
    let mut progress = 1u64;

    for p in positions {
        let account = account_id.to_owned();
        match p.instrument_type.as_str() {
            "bond" => {
                let paper = client
                    .create_paper_from_position(instruments, account, p, CouponProfit)
                    .await;
                add_paper_into_container(&mut container.bonds, paper);
            }
            "share" => {
                let paper = client
                    .create_paper_from_position(instruments, account, p, DividentProfit)
                    .await;
                add_paper_into_container(&mut container.shares, paper);
            }
            "etf" => {
                let paper = client
                    .create_paper_from_position(instruments, account, p, NoneProfit)
                    .await;
                add_paper_into_container(&mut container.etfs, paper);
            }
            "currency" => {
                let paper = client
                    .create_paper_from_position(instruments, account, p, NoneProfit)
                    .await;
                add_paper_into_container(&mut container.currencies, paper);
            }
            "futures" => {
                let paper = client
                    .create_paper_from_position(instruments, account, p, NoneProfit)
                    .await;
                add_paper_into_container(&mut container.futures, paper);
            }
            _ => {}
        };
        progresser.progress(progress);
        progress += 1;
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
