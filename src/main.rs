use std::{collections::HashMap, env};

use clap::{command, ArgAction, Command};
use itertools::Itertools;
use tinkoff::{
    client::{to_influence, OperationInfluence, TinkoffClient},
    domain::{Asset, Instrument, Money, Paper, Portfolio},
    progress::{Progress, Progresser},
    to_decimal, to_money, ux,
};
use tinkoff_invest_api::{tcs::AccountType, TIResult};

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

#[tokio::main]
async fn main() -> TIResult<()> {
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
        _ => Ok(()),
    }
}

async fn all(token: String, verbose: bool) -> TIResult<()> {
    let client = TinkoffClient::new(token);

    let (bonds, shares, etfs, currencies, portfolio) = tokio::join!(
        client.get_all_bonds(),
        client.get_all_shares(),
        client.get_all_etfs(),
        client.get_all_currencies(),
        client.get_portfolio_until_done(AccountType::Tinkoff),
    );

    let bonds = bonds?;
    let shares = shares?;
    let etfs = etfs?;
    let currencies = currencies?;

    let mut pf = Portfolio::new(verbose);
    let mut progresser = Progresser::new(portfolio.positions.len() as u64);
    let mut progress = 1u64;
    for p in &portfolio.positions {
        let Some(currency) = tinkoff::to_currency(&p.current_price) else {
            progresser.progress(progress);
            progress += 1;
            continue;
        };

        let expected_yield = to_decimal(p.expected_yield.as_ref());
        let expected_yield = Money::from_value(expected_yield, currency);
        let average_buy_price = to_money(p.average_position_price.as_ref()).unwrap();

        let quantity = to_decimal(p.quantity.as_ref());
        let balance_value = Money {
            value: average_buy_price.value * quantity,
            currency: average_buy_price.currency,
        };

        let current_instrument_price = to_money(p.current_price.as_ref()).unwrap();
        let current_value = Money {
            value: current_instrument_price.value * quantity,
            currency: current_instrument_price.currency,
        };

        let executed_ops = client
            .get_operations_until_done(portfolio.account_id.clone(), p.figi.clone())
            .await;

        let mut fees = Money::zero(currency);
        let mut dividents = Money::zero(currency);
        for op in &executed_ops {
            let op_type = op.operation_type();
            let Some(payment) = to_money(op.payment.as_ref()) else {
                continue;
            };
            match to_influence(op_type) {
                OperationInfluence::PureIncome => {
                    dividents.value += payment.value;
                }
                OperationInfluence::Fees => {
                    fees.value += payment.value;
                }
                OperationInfluence::Unspecified => {}
            }
        }

        let mut paper = Paper {
            name: String::new(),
            ticker: String::new(),
            figi: p.figi.clone(),
            expected_yield,
            average_buy_price,
            quantity,
            balance_value,
            current_value,
            current_instrument_price,
            taxes_and_fees: fees,
            dividents_and_coupons: dividents,
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
            _ => {}
        };
        progresser.progress(progress);
        progress += 1;
    }
    progresser.finish();
    print!("{pf}");

    Ok(())
}

async fn bonds(token: String) -> TIResult<()> {
    let client = TinkoffClient::new(token);
    let bonds = client.get_all_bonds().await?;
    let i = instruments!(bonds);
    asset(client, "Bonds".to_owned(), "bond", i).await
}

async fn shares(token: String) -> TIResult<()> {
    let client = TinkoffClient::new(token);
    let shares = client.get_all_shares().await?;
    let i = instruments!(shares);
    asset(client, "Shares".to_owned(), "share", i).await
}

async fn etfs(token: String) -> TIResult<()> {
    let client = TinkoffClient::new(token);
    let etfs = client.get_all_etfs().await?;
    let i = instruments!(etfs);
    asset(client, "Etfs".to_owned(), "etf", i).await
}

async fn currencies(token: String) -> TIResult<()> {
    let client = TinkoffClient::new(token);
    let currencies = client.get_all_currencies().await?;
    let i = instruments!(currencies);
    asset(client, "Currencies".to_owned(), "currency", i).await
}

async fn asset(
    client: TinkoffClient,
    asset_name: String,
    instrument_type: &str,
    instruments: HashMap<String, Instrument>,
) -> TIResult<()> {
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
        let Some(currency) = tinkoff::to_currency(&p.current_price) else {
            progresser.progress(progress);
            progress += 1;
            continue;
        };

        let expected_yield = to_decimal(p.expected_yield.as_ref());
        let expected_yield = Money::from_value(expected_yield, currency);
        let average_buy_price = to_money(p.average_position_price.as_ref()).unwrap();

        let quantity = to_decimal(p.quantity.as_ref());
        let balance_value = Money {
            value: average_buy_price.value * quantity,
            currency: average_buy_price.currency,
        };

        let current_instrument_price = to_money(p.current_price.as_ref()).unwrap();
        let current_value = Money {
            value: current_instrument_price.value * quantity,
            currency: current_instrument_price.currency,
        };

        let executed_ops = client
            .get_operations_until_done(portfolio.account_id.clone(), p.figi.clone())
            .await;

        let mut fees = Money::zero(currency);
        let mut dividents = Money::zero(currency);
        for op in &executed_ops {
            let op_type = op.operation_type();
            let Some(payment) = to_money(op.payment.as_ref()) else {
                continue;
            };
            match to_influence(op_type) {
                OperationInfluence::PureIncome => {
                    dividents.value += payment.value;
                }
                OperationInfluence::Fees => {
                    fees.value += payment.value;
                }
                OperationInfluence::Unspecified => {}
            }
        }

        if let Some(inst) = instruments.get(&p.figi) {
            let paper = Paper {
                name: inst.name.clone(),
                ticker: inst.ticker.clone(),
                figi: p.figi.clone(),
                expected_yield,
                average_buy_price,
                quantity,
                balance_value,
                current_value,
                current_instrument_price,
                taxes_and_fees: fees,
                dividents_and_coupons: dividents,
            };

            asset.add_paper(paper);
            progresser.progress(progress);
            progress += 1;
        }
    }
    progresser.finish();
    println!("{asset}");

    Ok(())
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
