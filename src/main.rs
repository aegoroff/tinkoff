use std::env;

use prettytable::{cell, row, Row, Table};
use tinkoff::{
    client::TinkoffClient,
    domain::{Money, Paper, Portfolio},
    to_decimal, to_money, ux,
};
use tinkoff_invest_api::{tcs::AccountType, TIResult};

#[tokio::main]
async fn main() -> TIResult<()> {
    let token = env::var("TINKOFF_TOKEN_V2");
    let client = TinkoffClient::new(token.unwrap());

    let (bonds, shares, etfs, currencies, portfolio) = tokio::join!(
        client.get_all_bonds(),
        client.get_all_shares(),
        client.get_all_etfs(),
        client.get_all_currencies(),
        client.get_portfolio(AccountType::Tinkoff),
    );

    let bonds = bonds?;
    let shares = shares?;
    let etfs = etfs?;
    let currencies = currencies?;
    let portfolio = portfolio?;

    let mut pf = Portfolio::new();
    for p in &portfolio.positions {
        let Some(currency) = iso_currency::Currency::from_code(
            &p.current_price
                .as_ref()
                .unwrap()
                .currency
                .to_ascii_uppercase(),
        ) else {
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
            .get_operations(portfolio.id.clone(), p.figi.clone())
            .await?;

        let mut fees = Money::zero(currency);
        let mut dividents = Money::zero(currency);
        for op in &executed_ops {
            let op_type = op.operation_type();
            let Some(payment) = to_money(op.payment.as_ref()) else {
                continue;
            };
            match op_type {
                tinkoff_invest_api::tcs::OperationType::DividendTax
                | tinkoff_invest_api::tcs::OperationType::BondTax
                | tinkoff_invest_api::tcs::OperationType::Coupon
                | tinkoff_invest_api::tcs::OperationType::Dividend => {
                    dividents.value += payment.value;
                }
                tinkoff_invest_api::tcs::OperationType::ServiceFee
                | tinkoff_invest_api::tcs::OperationType::BenefitTax
                | tinkoff_invest_api::tcs::OperationType::MarginFee
                | tinkoff_invest_api::tcs::OperationType::BrokerFee
                | tinkoff_invest_api::tcs::OperationType::SuccessFee
                | tinkoff_invest_api::tcs::OperationType::TrackMfee
                | tinkoff_invest_api::tcs::OperationType::TrackPfee
                | tinkoff_invest_api::tcs::OperationType::CashFee
                | tinkoff_invest_api::tcs::OperationType::OutFee
                | tinkoff_invest_api::tcs::OperationType::OutStampDuty
                | tinkoff_invest_api::tcs::OperationType::AdviceFee
                | tinkoff_invest_api::tcs::OperationType::Tax
                | tinkoff_invest_api::tcs::OperationType::OutputPenalty => {
                    fees.value += payment.value;
                }
                _ => {}
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
                let b = bonds.get(&p.figi).unwrap();
                paper.name = b.name.clone();
                paper.ticker = b.ticker.clone();
                pf.bonds.add_paper(paper);
            }
            "share" => {
                let s = shares.get(&p.figi).unwrap();
                paper.name = s.name.clone();
                paper.ticker = s.ticker.clone();
                pf.shares.add_paper(paper);
            }
            "etf" => {
                let e = etfs.get(&p.figi).unwrap();
                paper.name = e.name.clone();
                paper.ticker = e.ticker.clone();
                pf.etfs.add_paper(paper);
            }
            "currency" => {
                let c = currencies.get(&p.figi).unwrap();
                paper.name = c.name.clone();
                paper.ticker = c.ticker.clone();
                pf.currencies.add_paper(paper);
            }
            _ => {}
        };
    }
    pf.bonds.printstd();
    pf.shares.printstd();
    pf.currencies.printstd();

    let mut income = pf.bonds.income();
    income.add(&pf.shares.income());
    income.add(&pf.currencies.income());

    let mut assets = pf.bonds.current();
    assets.add(&pf.shares.current());
    assets.add(&pf.currencies.current());

    let income = ux::colored_cell(income);
    let mut table = Table::new();
    table.set_format(ux::new_table_format());

    table.set_titles(row![bFrH2 => "Portfolio totals:", ""]);
    table.add_row(Row::new(vec![cell!("Income"), income]));
    table.add_row(row!["Portfolio size", assets]);

    println!();
    println!();
    table.printstd();
    println!();

    Ok(())
}
