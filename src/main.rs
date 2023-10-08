use std::{collections::HashMap, env};

use clap::{command, ArgAction, Command};
use color_eyre::eyre::Result;
use itertools::Itertools;
use tinkoff::{
    client::TinkoffInvestment,
    domain::{Asset, Instrument, Paper, Portfolio, Position},
    progress::{Progress, Progresser},
    ux,
};
use tinkoff_invest_api::tcs::AccountType;

#[macro_use]
extern crate clap;

macro_rules! instruments {
    ($i:ident) => {{
        $i.iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    Instrument {
                        name: v.name.clone(),
                        ticker: v.ticker.clone(),
                    },
                )
            })
            .collect::<HashMap<String, Instrument>>()
    }};
}

macro_rules! add_instrument {
    ($container:ident, $paper:ident, $p:ident, $pf:ident, $target:ident) => {{
        if let Some(b) = $container.get(&$p.figi) {
            $paper.name = b.name.clone();
            $paper.ticker = b.ticker.clone();
            $pf.$target.add_paper($paper);
        }
    }};
}

macro_rules! impl_instrument_fn {
    ($(($name:ident, $method:ident, $asset_name:literal, $insr_type:literal)),*) => {
        $(
            async fn $name(token: String) {
                let client = TinkoffInvestment::new(token);
                let instruments = client.$method().await;
                let i = instruments!(instruments);
                asset(client, $asset_name.to_owned(), $insr_type, i).await;
            }
        )*
    };
}

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
        Some(("a", cmd)) => all(token, !cmd.get_flag("aggregate")).await,
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

    let (bonds, shares, etfs, currencies, futures, portfolio) = tokio::join!(
        client.get_all_bonds_until_done(),
        client.get_all_shares_until_done(),
        client.get_all_etfs_until_done(),
        client.get_all_currencies_until_done(),
        client.get_all_futures_until_done(),
        client.get_portfolio_until_done(AccountType::Tinkoff),
    );

    let mut pf = Portfolio::new(output_papers);
    let mut progresser = Progresser::new(portfolio.positions.len() as u64);
    let mut progress = 1u64;
    for p in &portfolio.positions {
        let Ok(position) = Position::try_from(p) else {
            progresser.progress(progress);
            progress += 1;
            continue;
        };

        let executed_ops = client
            .get_operations_until_done(portfolio.account_id.clone(), p.figi.clone())
            .await;

        let totals = tinkoff::client::reduce(&executed_ops, position.balance.currency);

        let mut paper = Paper {
            name: String::new(),
            ticker: String::new(),
            figi: p.figi.clone(),
            position,
            totals,
        };

        match p.instrument_type.as_str() {
            "bond" => {
                add_instrument!(bonds, paper, p, pf, bonds);
            }
            "share" => {
                add_instrument!(shares, paper, p, pf, shares);
            }
            "etf" => {
                add_instrument!(etfs, paper, p, pf, etfs);
            }
            "currency" => {
                add_instrument!(currencies, paper, p, pf, currencies);
            }
            "futures" => {
                add_instrument!(futures, paper, p, pf, futures);
            }
            _ => {}
        };
        progresser.progress(progress);
        progress += 1;
    }
    progresser.finish();
    print!("{pf}");
}

impl_instrument_fn!(
    (bonds, get_all_bonds_until_done, "Bonds", "bond"),
    (shares, get_all_shares_until_done, "Shares", "share"),
    (etfs, get_all_etfs_until_done, "Etfs", "etf"),
    (futures, get_all_futures_until_done, "Futures", "futures"),
    (
        currencies,
        get_all_currencies_until_done,
        "Currencies",
        "currency"
    )
);

async fn asset(
    client: TinkoffInvestment,
    asset_name: String,
    instrument_type: &str,
    instruments: HashMap<String, Instrument>,
) {
    let portfolio = client.get_portfolio_until_done(AccountType::Tinkoff).await;

    let positions = portfolio
        .positions
        .into_iter()
        .filter(|p| p.instrument_type == instrument_type)
        .collect_vec();

    let mut progresser = Progresser::new(positions.len() as u64);
    let mut progress = 1u64;
    let mut asset = Asset::new(asset_name.clone(), true);
    for p in &positions {
        let Ok(position) = Position::try_from(p) else {
            progresser.progress(progress);
            progress += 1;
            continue;
        };

        let executed_ops = client
            .get_operations_until_done(portfolio.account_id.clone(), p.figi.clone())
            .await;

        let totals = tinkoff::client::reduce(&executed_ops, position.balance.currency);

        if let Some(inst) = instruments.get(&p.figi) {
            let paper = Paper {
                name: inst.name.clone(),
                ticker: inst.ticker.clone(),
                figi: p.figi.clone(),
                position,
                totals,
            };

            asset.add_paper(paper);
            progresser.progress(progress);
            progress += 1;
        }
    }
    progresser.finish();
    println!("{asset}");
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
