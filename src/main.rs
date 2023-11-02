use std::{collections::HashMap, env};

use chrono::prelude::*;
use clap::{command, ArgAction, ArgMatches, Command};
use color_eyre::eyre::Result;
use iso_currency::Currency;
use itertools::Itertools;
use tinkoff::{
    client::TinkoffInvestment,
    domain::{
        Asset, CouponProfit, DivdentProfit, Instrument, Money, NoneProfit, Paper, Portfolio, Profit,
    },
    progress::{Progress, Progresser},
    to_money, ux,
};
use tinkoff_invest_api::tcs::{AccountType, PortfolioPosition};

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
        Some(("hi", cmd)) => history(token, cmd).await,
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
    let ticker = if let Some(t) = cmd.get_one::<String>("TICKER") {
        t.clone()
    } else {
        String::new()
    };
    let (account, instruments) = tokio::join!(
        client.get_account(AccountType::Tinkoff),
        client.find_instruments_by_ticker(ticker),
    );
    let account = account.unwrap();
    let instruments = instruments.unwrap();

    let mut operations = vec![];
    for instr in instruments {
        let instr_operaions = client
            .get_operations_until_done(account.id.clone(), instr.figi)
            .await;

        operations.extend(instr_operaions.iter().cloned());
    }
    operations
        .iter()
        .unique_by(|op| &op.id)
        .sorted_by(|a, b| {
            let dt_a = a.date.as_ref().unwrap();
            let dt_a = DateTime::<Utc>::from_timestamp(dt_a.seconds, dt_a.nanos as u32)
                .unwrap_or_default();
            let dt_b = b.date.as_ref().unwrap();
            let dt_b = DateTime::<Utc>::from_timestamp(dt_b.seconds, dt_b.nanos as u32)
                .unwrap_or_default();
            Ord::cmp(&dt_a, &dt_b)
        })
        .for_each(|op| {
            let currency = Currency::from_code(&op.currency.to_ascii_uppercase()).unwrap();
            let payment = if let Some(payment) = to_money(op.payment.as_ref()) {
                payment
            } else {
                Money::zero(currency)
            };
            let price = if let Some(price) = to_money(op.price.as_ref()) {
                price
            } else {
                Money::zero(currency)
            };

            let dt = op.date.as_ref().unwrap();
            let dt =
                DateTime::<Utc>::from_timestamp(dt.seconds, dt.nanos as u32).unwrap_or_default();

            println!(
                "{} | {} | {} | {} | {} | {:#?}",
                dt,
                op.quantity,
                price,
                payment,
                op.r#type,
                op.state()
            );
        });
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
                    .create_paper_from_position(instruments, account, p, DivdentProfit)
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

fn history_cmd() -> Command {
    Command::new("hi")
        .aliases(["history"])
        .about("Get an instrument history")
        .arg(arg!([TICKER]).help("Instrument's tiker").required(true))
}
