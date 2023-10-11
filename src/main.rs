use std::{collections::HashMap, env};

use clap::{command, ArgAction, Command};
use color_eyre::eyre::Result;
use itertools::Itertools;
use tinkoff::{
    client::TinkoffInvestment,
    domain::{
        Asset, CouponProfit, DivdentProfit, Instrument, NoneProfit, Paper, Portfolio,
        Profit,
    },
    progress::{Progress, Progresser},
    ux,
};
use tinkoff_invest_api::tcs::AccountType;

#[macro_use]
extern crate clap;

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;
    ux::clear_screen();
    let cli = build_cli().get_matches();

    let token = if let Some(t) = cli.get_one::<String>("token") {
        t.clone()
    } else {
        env::var("TINKOFF_TOKEN_V2").unwrap()
    };

    match cli.subcommand() {
        Some(("a", cmd)) => Box::pin(all(token, !cmd.get_flag("aggregate"))).await,
        Some(("s", _)) => shares(token).await,
        Some(("b", _)) => bonds(token).await,
        Some(("e", _)) => etfs(token).await,
        Some(("c", _)) => currencies(token).await,
        Some(("f", _)) => futures(token).await,
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

    let mut container = Portfolio::new(output_papers);
    let mut progresser = Progresser::new(portfolio.positions.len() as u64);
    let mut progress = 1u64;
    for p in &portfolio.positions {
        let account = portfolio.account_id.clone();
        match p.instrument_type.as_str() {
            "bond" => {
                let paper =
                client.create_paper_from_position(&all, account, p, CouponProfit).await;
                add_paper(&mut container.bonds, paper);
            }
            "share" => {
                let paper =
                client.create_paper_from_position(&all, account, p, DivdentProfit).await;
                add_paper(&mut container.shares, paper);
            }
            "etf" => {
                let paper = client.create_paper_from_position(&all, account, p, NoneProfit).await;
                add_paper(&mut container.etfs, paper);
            }
            "currency" => {
                let paper = client.create_paper_from_position(&all, account, p, NoneProfit).await;
                add_paper(&mut container.currencies, paper);
            }
            "futures" => {
                let paper = client.create_paper_from_position(&all, account, p, NoneProfit).await;
                add_paper(&mut container.futures, paper);
            }
            _ => {}
        };
        progresser.progress(progress);
        progress += 1;
    }
    progresser.finish();
    print!("{container}");
}

async fn bonds(token: String) {
    let client = TinkoffInvestment::new(token);
    let instruments = client.get_all_bonds_until_done().await;
    asset(client, "Bonds", "bond", instruments, CouponProfit).await;
}

async fn shares(token: String) {
    let client = TinkoffInvestment::new(token);
    let instruments = client.get_all_shares_until_done().await;
    asset(client, "Shares", "share", instruments, DivdentProfit).await;
}

async fn etfs(token: String) {
    let client = TinkoffInvestment::new(token);
    let instruments = client.get_all_etfs_until_done().await;
    asset(client, "Etfs", "etf", instruments, NoneProfit).await;
}

async fn futures(token: String) {
    let client = TinkoffInvestment::new(token);
    let instruments = client.get_all_futures_until_done().await;
    asset(client, "Futures", "futures", instruments, NoneProfit).await;
}

async fn currencies(token: String) {
    let client = TinkoffInvestment::new(token);
    let instruments = client.get_all_currencies_until_done().await;
    asset(client, "Currencies", "currency", instruments, NoneProfit).await;
}

async fn asset<P: Profit>(
    client: TinkoffInvestment,
    asset_name: &'static str,
    instrument_type: &str,
    instruments: HashMap<String, Instrument>,
    profit: P,
) {
    let portfolio = client.get_portfolio_until_done(AccountType::Tinkoff).await;

    let positions = portfolio
        .positions
        .into_iter()
        .filter(|p| p.instrument_type == instrument_type)
        .collect_vec();

    let mut progresser = Progresser::new(positions.len() as u64);
    let mut progress = 1u64;
    let mut asset = Asset::new(asset_name, profit, true);
    for p in &positions {
        let paper = client
            .create_paper_from_position(
                &instruments,
                portfolio.account_id.clone(),
                p,
                profit,
            )
            .await;

        add_paper(&mut asset, paper);

        progresser.progress(progress);
        progress += 1;
    }
    progresser.finish();
    println!("{asset}");
}

fn add_paper<P: Profit>(asset: &mut Asset<P>, paper: Option<Paper<P>>) {
    if let Some(p) = paper {
        asset.add_paper(p);
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
        .subcommand(all_cmd())
        .subcommand(shares_cmd())
        .subcommand(bonds_cmd())
        .subcommand(etfs_cmd())
        .subcommand(currencies_cmd())
        .subcommand(futures_cmd())
}

fn all_cmd() -> Command {
    Command::new("a")
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
    Command::new("s")
        .aliases(["shares"])
        .about("Get portfolio shares")
}

fn bonds_cmd() -> Command {
    Command::new("b")
        .aliases(["bonds"])
        .about("Get portfolio bonds")
}

fn etfs_cmd() -> Command {
    Command::new("e")
        .aliases(["etfs"])
        .about("Get portfolio etfs")
}

fn currencies_cmd() -> Command {
    Command::new("c")
        .aliases(["currencies"])
        .about("Get portfolio currencies")
}

fn futures_cmd() -> Command {
    Command::new("f")
        .aliases(["futures"])
        .about("Get portfolio futures")
}
