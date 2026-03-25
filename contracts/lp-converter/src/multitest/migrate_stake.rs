use cosmwasm_std::testing::MockApi;
use cosmwasm_std::Uint256;
use wyndex::{asset::MINIMUM_LIQUIDITY_AMOUNT, stake::ConverterConfig};
use wyndex_stake::msg::MigrateMsg;

use super::suite::{juno, uusd, Pair, SuiteBuilder, DAY};

#[test]
fn migrate_to_existing_pool() {
    let user = MockApi::default().addr_make("user");

    let ujuno_amount = 1_000_000u128;
    let lsd_amount = 1_000_000u128;
    let uusd_amount = 1_000_000u128;

    let unbonding_period = 14 * DAY;

    let mut suite = SuiteBuilder::new()
        .with_native_balances(
            "ujuno",
            vec![(&user.to_string(), lsd_amount + ujuno_amount)],
        )
        .with_native_balances("uusd", vec![(&user.to_string(), 2 * uusd_amount)])
        .build();

    // get some wyJUNO
    suite.bond_juno(&user, lsd_amount).unwrap();
    let lsd_balance = suite.query_cw20_balance(&user, &suite.lsd_token).unwrap();
    assert_eq!(lsd_balance, lsd_amount);

    // provide some base liquidity to both pools
    let native_lp = suite
        .provide_liquidity(&user, juno(ujuno_amount), uusd(uusd_amount))
        .unwrap();
    let lsd_lp = suite
        .provide_liquidity(&user, suite.lsd_asset(lsd_amount), uusd(uusd_amount))
        .unwrap();

    // stake native LP
    suite
        .stake_lp(Pair::Native, &user, native_lp, unbonding_period)
        .unwrap();
    let stake = suite
        .query_stake(Pair::Native, &user, unbonding_period)
        .unwrap();
    assert_eq!(
        stake.stake,
        Uint256::from(ujuno_amount) - MINIMUM_LIQUIDITY_AMOUNT
    );

    // migrating from lsd LP to native LP should fail
    let err = suite
        .migrate_stake(Pair::Lsd, &user, lsd_lp, unbonding_period)
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("Cannot migrate stake without a converter contract"),
        "unexpected error: {err}"
    );

    // migrate it to lsd LP
    suite
        .migrate_stake(Pair::Native, &user, native_lp, unbonding_period)
        .unwrap();

    // check that the stake was migrated
    let stake = suite
        .query_stake(Pair::Native, &user, unbonding_period)
        .unwrap();
    assert_eq!(stake.stake, Uint256::zero());
    let stake = suite
        .query_stake(Pair::Lsd, &user, unbonding_period)
        .unwrap();
    assert_eq!(
        stake.stake,
        Uint256::new(ujuno_amount) - MINIMUM_LIQUIDITY_AMOUNT,
        "all of the stake that was previously in native LP should now be migrated to lsd LP"
    );
}

#[test]
fn migrate_converter_config() {
    let user = MockApi::default().addr_make("user");

    let ujuno_amount = 1_000_000u128;
    let uusd_amount = 1_000_000u128;

    let unbonding_period = 14 * DAY;

    let mut suite = SuiteBuilder::new()
        .with_native_balances("ujuno", vec![(&user.to_string(), ujuno_amount)])
        .with_native_balances("uusd", vec![(&user.to_string(), uusd_amount)])
        .without_converter()
        .build();

    // provide some liquidity to the native pair
    let native_lp = suite
        .provide_liquidity(&user, juno(ujuno_amount), uusd(uusd_amount))
        .unwrap();

    // stake native LP
    suite
        .stake_lp(Pair::Native, &user, native_lp, unbonding_period)
        .unwrap();

    // migrating the liquidity before the converter is set should fail
    let err = suite
        .migrate_stake(Pair::Native, &user, native_lp, unbonding_period)
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("Cannot migrate stake without a converter contract"),
        "unexpected error: {err}"
    );

    // migrate the staking contract to add the converter
    suite
        .migrate_staking_contract(
            Pair::Native,
            MigrateMsg {
                unbonder: None,
                converter: Some(ConverterConfig {
                    contract: suite.converter.to_string(),
                    pair_to: suite.lsd_pair.to_string(),
                }),
                unbond_all: false,
            },
        )
        .unwrap();

    // migrate liquidity to lsd pair
    suite
        .migrate_stake(Pair::Native, &user, native_lp, unbonding_period)
        .unwrap();

    // check that the stake was migrated
    let stake = suite
        .query_stake(Pair::Native, &user, unbonding_period)
        .unwrap();
    assert_eq!(stake.stake, Uint256::zero());
    let stake = suite
        .query_stake(Pair::Lsd, &user, unbonding_period)
        .unwrap();
    assert_eq!(
        stake.stake,
        Uint256::from(ujuno_amount) - Uint256::from(2u128) * MINIMUM_LIQUIDITY_AMOUNT, // 2x because we lp'd twice on empty pools
        "all of the stake that was previously in native LP should now be migrated to lsd LP"
    );
}

#[test]
fn partial_migration() {
    let user = MockApi::default().addr_make("user");

    let ujuno_amount = 1_000_000u128;
    let uusd_amount = 1_000_000u128;

    let unbonding_period = 14 * DAY;

    let mut suite = SuiteBuilder::new()
        .with_native_balances("ujuno", vec![(&user.to_string(), ujuno_amount)])
        .with_native_balances("uusd", vec![(&user.to_string(), 2 * uusd_amount)])
        .build();

    // provide some base liquidity to native pool
    let native_lp = suite
        .provide_liquidity(&user, juno(ujuno_amount), uusd(uusd_amount))
        .unwrap();

    // stake native LP
    suite
        .stake_lp(Pair::Native, &user, native_lp, unbonding_period)
        .unwrap();

    // migrate half of native LP to lsd LP
    suite
        .migrate_stake(Pair::Native, &user, native_lp / 2, unbonding_period)
        .unwrap();

    // check that only half of the stake was migrated
    let stake = suite
        .query_stake(Pair::Native, &user, unbonding_period)
        .unwrap();
    assert_eq!(
        stake.stake,
        (Uint256::from(ujuno_amount) - MINIMUM_LIQUIDITY_AMOUNT) / Uint256::from(2u128),
        "half of the stake (minus minimum amount) should remain in native LP"
    );
    let stake = suite
        .query_stake(Pair::Lsd, &user, unbonding_period)
        .unwrap();
    assert_eq!(
        stake.stake,
        (Uint256::from(ujuno_amount) - MINIMUM_LIQUIDITY_AMOUNT) / Uint256::from(2u128)
            - MINIMUM_LIQUIDITY_AMOUNT,
        "half of the stake should be migrated to lsd LP (minus minimum amount)"
    );
}

#[test]
fn empty_stake_fails() {
    let user = MockApi::default().addr_make("user");

    let ujuno_amount = 1_000_000u128;
    let uusd_amount = 1_000_000u128;

    let unbonding_period = 14 * DAY;

    let mut suite = SuiteBuilder::new()
        .with_native_balances("ujuno", vec![(&user.to_string(), ujuno_amount)])
        .with_native_balances("uusd", vec![(&user.to_string(), uusd_amount)])
        .build();

    // provide some base liquidity to native pool
    let native_lp = suite
        .provide_liquidity(&user, juno(ujuno_amount), uusd(uusd_amount))
        .unwrap();

    // stake native LP
    suite
        .stake_lp(Pair::Native, &user, native_lp, unbonding_period)
        .unwrap();
    let stake = suite
        .query_stake(Pair::Native, &user, unbonding_period)
        .unwrap();
    assert_eq!(
        stake.stake,
        Uint256::from(ujuno_amount) - MINIMUM_LIQUIDITY_AMOUNT
    );

    // // migrating zero amount should fail
    // let err = suite
    //     .migrate_stake(Pair::Native, user, 0, unbonding_period)
    //     .unwrap_err();
    // assert_eq!(
    //     cw20_base::ContractError::InvalidZeroAmount {},
    //     err.downcast().unwrap()
    // );

    // migrating more stake than available should fail
    suite
        .migrate_stake(Pair::Native, &user, ujuno_amount, unbonding_period)
        .unwrap_err();

    // check that the stake was not migrated
    let stake = suite
        .query_stake(Pair::Native, &user, unbonding_period)
        .unwrap();
    assert_eq!(
        stake.stake,
        Uint256::from(ujuno_amount) - MINIMUM_LIQUIDITY_AMOUNT
    );
}
