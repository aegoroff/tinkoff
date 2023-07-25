use std::env;

use tinkoff::{
    client::{to_influence, OperationInfluence, TinkoffClient},
    domain::{Money, Paper, Portfolio},
    progress::{Progress, Progresser},
    to_decimal, to_money,
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
    let mut progresser = Progresser::new(portfolio.positions.len() as u64);
    let mut progress = 1u64;
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
            .get_operations(portfolio.account_id.clone(), p.figi.clone())
            .await?;

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
        progresser.progress(progress);
        progress += 1;
    }
    progresser.finish("");
    pf.printstd();

    Ok(())
}
