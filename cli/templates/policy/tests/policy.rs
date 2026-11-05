use allowit::prelude::*;
use allowit_policy_template::execute;
use std::collections::BTreeMap;

fn context(amount: i128) -> AllowitContext {
    AllowitContext {
        env: allowit::AllowitEnv::new(),
        params: BTreeMap::from([
            (
                allowit::OPERATIONS.into(),
                AllowitValue::List(vec![AllowitValue::Str("transfer".into())].into()),
            ),
            (allowit::DAILY_SPENDING_LIMIT.into(), AllowitValue::Int(10)),
            (allowit::PAYMENT_LIMIT.into(), AllowitValue::Int(5)),
        ])
        .into(),
        state: BTreeMap::new(),
        request: AllowitRequest {
            operation: AllowitOperation::transfer,
            asset: Address::new("asset"),
            recipient: Address::new("recipient"),
            amount,
            token_in: Address::new("asset"),
            amount_in: amount,
            token_out: None,
            min_out: 0,
            now: allowit::DAY + 100,
            expires_at: allowit::DAY + 500,
            spent_units: amount,
            gas_units: 0,
            unpriced: false,
            args: BTreeMap::new().into(),
        },
    }
}

#[test]
fn starter_checks_allowed_operations_before_the_retained_spending_limits() {
    assert_eq!(execute(&context(1)), Ok(()));
    for amount in [1, 11] {
        let mut other = context(amount);
        other.request.operation = AllowitOperation::swap;
        assert_eq!(execute(&other), Err(AllowitError::OperationNotAllowed));
    }
    assert_eq!(
        execute(&context(6)),
        Err(AllowitError::PaymentLimitExceeded)
    );
    assert_eq!(
        execute(&context(11)),
        Err(AllowitError::DailySpendingLimitExceeded)
    );
}
