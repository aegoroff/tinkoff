use std::{collections::HashMap, env};

use clap::{command, ArgAction, Command};
use color_eyre::eyre::Result;
use itertools::Itertools;
use tinkoff::{
    client::TinkoffInvestment,
    domain::{
        Asset, CouponProfit, DivdentProfit, Instrument, NoneProfit, Paper, Portfolio, Position,
        Profit, Totals,
    },
    progress::{Progress, Progresser},
    ux,
};
use tinkoff_invest_api::tcs::AccountType;

#[macro_use]
extern crate clap;

macro_rules! impl_instrument_fn {
    ($(($name:ident, $method:ident, $asset_name:literal, $insr_type:literal, $profit:ident)),*) => {
        $(
            async fn $name(token: String) {
                let client = TinkoffInvestment::new(token);
                let instruments = client.$method().await;
                asset(client, $asset_name, $insr_type, instruments, $profit).await;
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

        let totals = tinkoff::client::reduce(&executed_ops, position.currency);

        match p.instrument_type.as_str() {
            "bond" => {
                let paper = create_paper(&all, position, totals, &p.figi, CouponProfit);
                add_paper(&mut pf.bonds, paper);
            }
            "share" => {
                let paper = create_paper(&all, position, totals, &p.figi, DivdentProfit);
                add_paper(&mut pf.shares, paper);
            }
            "etf" => {
                let paper = create_paper(&all, position, totals, &p.figi, NoneProfit);
                add_paper(&mut pf.etfs, paper);
            }
            "currency" => {
                let paper = create_paper(&all, position, totals, &p.figi, NoneProfit);
                add_paper(&mut pf.currencies, paper);
            }
            "futures" => {
                let paper = create_paper(&all, position, totals, &p.figi, NoneProfit);
                add_paper(&mut pf.futures, paper);
            }
            _ => {}
        };
        progresser.progress(progress);
        progress += 1;
    }
    progresser.finish();
    print!("{pf}");
}

fn create_paper<P: Profit>(
    instruments: &HashMap<String, Instrument>,
    position: Position,
    totals: Totals,
    figi: &String,
    profit: P,
) -> Option<Paper<P>> {
    let b = instruments.get(figi)?;
    Some(Paper {
        name: b.name.clone(),
        ticker: b.ticker.clone(),
        figi: figi.clone(),
        position,
        totals,
        profit,
    })
}

fn add_paper<P: Profit>(asset: &mut Asset<P>, paper: Option<Paper<P>>) {
    if let Some(p) = paper {
        asset.add_paper(p);
    }
}

impl_instrument_fn!(
    (
        bonds,
        get_all_bonds_until_done,
        "Bonds",
        "bond",
        CouponProfit
    ),
    (
        shares,
        get_all_shares_until_done,
        "Shares",
        "share",
        DivdentProfit
    ),
    (etfs, get_all_etfs_until_done, "Etfs", "etf", NoneProfit),
    (
        futures,
        get_all_futures_until_done,
        "Futures",
        "futures",
        NoneProfit
    ),
    (
        currencies,
        get_all_currencies_until_done,
        "Currencies",
        "currency",
        NoneProfit
    )
);

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
        let Ok(position) = Position::try_from(p) else {
            progresser.progress(progress);
            progress += 1;
            continue;
        };

        let executed_ops = client
            .get_operations_until_done(portfolio.account_id.clone(), p.figi.clone())
            .await;

        let totals = tinkoff::client::reduce(&executed_ops, position.currency);

        let paper = create_paper(&instruments, position, totals, &p.figi, profit);
        add_paper(&mut asset, paper);

        progresser.progress(progress);
        progress += 1;
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
