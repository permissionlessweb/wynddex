use cosmwasm_std::testing::MockApi;
use cosmwasm_std::{
    Addr, Decimal256, OverflowError, OverflowOperation, StdError, Uint128, Uint256,
};
use wyndex::asset::{AssetInfo, AssetInfoValidated};
use wyndex::stake::UnbondingPeriod;
use wyndex_test_helpers::TestAccounts;

use crate::error::ContractError;
use crate::msg::{AllStakedResponse, StakedResponse};
use crate::multitest::suite::{juno_power, SEVEN_DAYS};

use super::suite::SuiteBuilder;
use test_case::test_case;

#[test]
fn unbond_overflow() {
    let unbonding_period = 1000u64;
    let mut suite = SuiteBuilder::new()
        .with_unbonding_periods(vec![(unbonding_period)])
        .build();

    let user = suite.app.api().addr_make("user");

    let err = suite.unbond(&user, 1u128, unbonding_period).unwrap_err();
    assert!(err.to_string().contains("Cannot Sub with given operands"));
}

#[test]
fn no_unbonding_period_found() {
    let user1 = MockApi::default().addr_make("user");
    let unbonding_period = 1000u64;
    let mut suite = SuiteBuilder::new()
        .with_unbonding_periods(vec![unbonding_period])
        .with_initial_balances(vec![(&user1, 100_000)])
        .build();

    let err = suite
        .delegate(&user1, 12_000u128, unbonding_period + 1)
        .unwrap_err();
    assert!(err.to_string().contains(&ContractError::NoUnbondingPeriodFound(unbonding_period + 1).to_string()));

    suite
        .delegate(&user1, 12_000u128, unbonding_period)
        .unwrap();

    let err = suite
        .unbond(&user1, 12_000u128, unbonding_period + 1)
        .unwrap_err();
    assert!(err.to_string().contains(&ContractError::NoUnbondingPeriodFound(unbonding_period + 1).to_string()));

    suite.unbond(&user1, 12_000u128, unbonding_period).unwrap();
}

#[test]
fn one_user_multiple_unbonding_periods() {
    let user = MockApi::default().addr_make("user");
    let unbonding_period1 = 1000u64;
    let unbonding_period2 = 4000u64;
    let unbonding_period3 = 8000u64;
    let mut suite = SuiteBuilder::new()
        .with_unbonding_periods(vec![
            unbonding_period1,
            unbonding_period2,
            unbonding_period3,
        ])
        .with_initial_balances(vec![(&user, 100_000)])
        .build();

    let bonds = vec![20_000u128, 30_000u128, 10_000u128];
    let delegated: u128 = bonds.iter().sum();

    suite.delegate(&user, bonds[0], unbonding_period1).unwrap();
    suite.delegate(&user, bonds[1], unbonding_period2).unwrap();
    suite.delegate(&user, bonds[2], unbonding_period3).unwrap();

    assert_eq!(suite.query_balance_staking_contract().unwrap(), delegated);

    // unbond on second unbonding period
    suite.unbond(&user, 20_000u128, unbonding_period2).unwrap();
    assert_eq!(
        suite.query_staked(&user, unbonding_period2).unwrap(),
        10_000u128
    );

    // top some more on first unbonding period
    suite.delegate(&user, 5_000u128, unbonding_period1).unwrap();
    assert_eq!(
        suite.query_staked(&user, unbonding_period1).unwrap(),
        25_000u128
    );

    assert_eq!(
        suite.query_all_staked(&user).unwrap(),
        AllStakedResponse {
            stakes: vec![
                StakedResponse {
                    stake: Uint256::from(25_000u128),
                    total_locked: Uint256::zero(),
                    unbonding_period: 1000,
                    cw20_contract: suite.token_contract(),
                },
                StakedResponse {
                    stake: Uint256::from(10_000u128),
                    total_locked: Uint256::zero(),
                    unbonding_period: 4000,
                    cw20_contract: suite.token_contract(),
                },
                StakedResponse {
                    stake: Uint256::from(10_000u128),
                    total_locked: Uint256::zero(),
                    unbonding_period: 8000,
                    cw20_contract: suite.token_contract(),
                },
            ]
        }
    );

    let periods = suite.query_staked_periods().unwrap();
    assert_eq!(periods.len(), 3);
    assert_eq!(periods[0].unbonding_period, unbonding_period1);
    assert_eq!(
        Uint128::try_from(periods[0].total_staked).unwrap().u128(),
        25_000
    );
    assert_eq!(periods[1].unbonding_period, unbonding_period2);
    assert_eq!(
        Uint128::try_from(periods[1].total_staked).unwrap().u128(),
        10_000
    );
    assert_eq!(periods[2].unbonding_period, unbonding_period3);
    assert_eq!(
        Uint128::try_from(periods[2].total_staked).unwrap().u128(),
        10_000
    );
}

#[test]
fn one_user_multiple_periods_rebond_then_bond() {
    let user = MockApi::default().addr_make("user");
    let unbonding_period1 = 1000u64;
    let unbonding_period2 = 4000u64;
    let unbonding_period3 = 8000u64;
    let mut suite = SuiteBuilder::new()
        .with_unbonding_periods(vec![
            unbonding_period1,
            unbonding_period2,
            unbonding_period3,
        ])
        .with_admin(&MockApi::default().addr_make("admin"))
        .with_initial_balances(vec![(&user, 100_000)])
        .build();

    suite
        .create_distribution_flow(
            &MockApi::default().addr_make("admin"),
            &user,
            AssetInfo::Native("juno".to_string()),
            vec![
                (unbonding_period1, Decimal256::percent(25)),
                (unbonding_period2, Decimal256::percent(60)),
                (unbonding_period3, Decimal256::percent(80)),
            ],
        )
        .unwrap();

    let bonds = vec![20_000u128, 30_000u128, 10_000u128];
    let delegated: u128 = bonds.iter().sum();

    suite.delegate(&user, bonds[0], unbonding_period1).unwrap();
    suite.delegate(&user, bonds[1], unbonding_period2).unwrap();
    suite.delegate(&user, bonds[2], unbonding_period3).unwrap();

    assert_eq!(suite.query_balance_staking_contract().unwrap(), delegated);

    // rebond all tokens from bonding period 1 to period 2
    suite
        .rebond(&user, 20_000u128, unbonding_period1, unbonding_period2)
        .unwrap();
    assert_eq!(suite.query_staked(&user, unbonding_period1).unwrap(), 0u128);

    assert_eq!(
        suite.query_staked(&user, unbonding_period2).unwrap(),
        50_000u128
    );

    assert_eq!(
        suite.query_rewards_power(&user).unwrap(),
        vec![(AssetInfoValidated::Native("juno".to_string()), 38u128)]
    );
    assert_eq!(suite.query_total_rewards_power().unwrap(), juno_power(38)); // 0.25 * 0 + 0.6 * 50_000 + 0.8 * 10_000

    // top some more on first unbonding period but not more than we originally topped up
    suite
        .delegate(&user, 25_000u128, unbonding_period1)
        .unwrap();
    assert_eq!(
        suite.query_staked(&user, unbonding_period1).unwrap(),
        25_000u128
    );
    assert_eq!(
        suite.query_all_staked(&user).unwrap(),
        AllStakedResponse {
            stakes: vec![
                StakedResponse {
                    stake: Uint256::from(25_000u128),
                    total_locked: Uint256::zero(),
                    unbonding_period: 1000,
                    cw20_contract: suite.token_contract(),
                },
                StakedResponse {
                    stake: Uint256::from(50_000u128),
                    total_locked: Uint256::zero(),
                    unbonding_period: 4000,
                    cw20_contract: suite.token_contract(),
                },
                StakedResponse {
                    stake: Uint256::from(10_000u128),
                    total_locked: Uint256::zero(),
                    unbonding_period: 8000,
                    cw20_contract: suite.token_contract(),
                },
            ]
        }
    );
    assert_eq!(
        suite.query_rewards_power(&user).unwrap(),
        vec![(AssetInfoValidated::Native("juno".to_string()), 44u128)]
    );
    assert_eq!(suite.query_total_rewards_power().unwrap(), juno_power(44)); // 0.25 * 25_000 + 0.6 * 50_000 + 0.8 * 10_000
}

#[test]
fn rebond_then_rebond_again() {
    let user = MockApi::default().addr_make("user");
    let unbonding_period1 = 1000u64;
    let unbonding_period2 = 4000u64;
    let unbonding_period3 = 8000u64;
    let mut suite = SuiteBuilder::new()
        .with_unbonding_periods(vec![
            unbonding_period1,
            unbonding_period2,
            unbonding_period3,
        ])
        .with_initial_balances(vec![(&user, 100_000)])
        .build();

    // delegate on first unbonding period
    suite
        .delegate(&user, 100_000u128, unbonding_period1)
        .unwrap();
    assert_eq!(
        suite.query_staked(&user, unbonding_period1).unwrap(),
        100_000u128
    );

    // rebond 40% of tokens to bucket 2
    suite
        .rebond(&user, 40_000u128, unbonding_period1, unbonding_period2)
        .unwrap();
    assert_eq!(
        suite.query_staked(&user, unbonding_period1).unwrap(),
        60_000u128
    );

    assert_eq!(
        suite.query_staked(&user, unbonding_period2).unwrap(),
        40_000u128
    );

    // rebond half of bucket 2 tokens to bucket 3
    suite
        .rebond(&user, 20_000u128, unbonding_period2, unbonding_period3)
        .unwrap();
    assert_eq!(
        suite.query_staked(&user, unbonding_period2).unwrap(),
        20_000u128
    );

    assert_eq!(
        suite.query_staked(&user, unbonding_period3).unwrap(),
        20_000u128
    );

    assert_eq!(
        suite.query_all_staked(&user).unwrap(),
        AllStakedResponse {
            stakes: vec![
                StakedResponse {
                    stake: Uint256::from(60_000u128),
                    total_locked: Uint256::zero(),
                    unbonding_period: 1000,
                    cw20_contract: suite.token_contract(),
                },
                StakedResponse {
                    stake: Uint256::from(20_000u128),
                    total_locked: Uint256::zero(),
                    unbonding_period: 4000,
                    cw20_contract: suite.token_contract(),
                },
                StakedResponse {
                    stake: Uint256::from(20_000u128),
                    total_locked: Uint256::zero(),
                    unbonding_period: 8000,
                    cw20_contract: suite.token_contract(),
                },
            ]
        }
    );
}

#[test]
fn one_user_multiple_periods_rebond_fail() {
    let user = MockApi::default().addr_make("user");
    let unbonding_period1 = 1000u64;
    let unbonding_period2 = 4000u64;
    let unbonding_period3 = 8000u64;
    let mut suite = SuiteBuilder::new()
        .with_unbonding_periods(vec![
            unbonding_period1,
            unbonding_period2,
            unbonding_period3,
        ])
        .with_initial_balances(vec![(&user, 100_000)])
        .build();

    let bonds = vec![20_000u128, 30_000u128, 10_000u128];
    let delegated: u128 = bonds.iter().sum();

    suite.delegate(&user, bonds[0], unbonding_period1).unwrap();
    suite.delegate(&user, bonds[1], unbonding_period2).unwrap();
    suite.delegate(&user, bonds[2], unbonding_period3).unwrap();

    assert_eq!(suite.query_balance_staking_contract().unwrap(), delegated);

    // Fail case, rebonding 50_000 from a bucket with 20_000
    let err = suite
        .rebond(&user, 50_000u128, unbonding_period1, unbonding_period2)
        .unwrap_err();
    assert!(err.to_string().contains("Cannot Sub with given operands"));

    // Fail case, rebonding to a non-existent bucket
    let err = suite
        .rebond(&user, 10_000u128, unbonding_period1, 12000)
        .unwrap_err();
    assert!(err.to_string().contains(&ContractError::NoUnbondingPeriodFound(12000).to_string()));

    // Fail case, rebonding from a non-existent bucket
    let err = suite
        .rebond(&user, 50_000u128, 2000, unbonding_period2)
        .unwrap_err();
    assert!(err.to_string().contains(&ContractError::NoUnbondingPeriodFound(2000).to_string()));
}

#[test]
fn multiple_users_multiple_unbonding_periods() {
    let a = TestAccounts::new(&MockApi::default());

    let asset0000 = a.owner;
    let factory = a.beneficiary;
    let user1 = a.user;
    let user2 = a.trader;
    let user3 = a.whale;
    let unbonding_period1 = 1000u64;
    let unbonding_period2 = 4000u64;
    let unbonding_period3 = 8000u64;

    let bonds = vec![20_000u128, 30_000u128, 10_000u128, 16_000u128, 6_000u128];
    let delegated: u128 = bonds.iter().sum();
    let members = [user1.clone(), user2.clone(), user3.clone()];

    let mut suite = SuiteBuilder::new()
        .with_unbonding_periods(vec![
            unbonding_period1,
            unbonding_period2,
            unbonding_period3,
        ])
        .with_admin(&MockApi::default().addr_make("admin"))
        .with_min_bond(4_500)
        .with_initial_balances(vec![
            (&user1, 100_000),
            (&user2, 100_000),
            (&user3, 100_000),
        ])
        .build();

    suite
        .create_distribution_flow(
            &MockApi::default().addr_make("admin"),
            &members[0],
            AssetInfo::Native("juno".to_string()),
            vec![
                (unbonding_period1, Decimal256::percent(1)),
                (unbonding_period2, Decimal256::percent(40)),
                (unbonding_period3, Decimal256::percent(60)),
            ],
        )
        .unwrap();

    suite
        .delegate(&members[0], bonds[0], unbonding_period1)
        .unwrap();
    suite
        .delegate(&members[1], bonds[1], unbonding_period2)
        .unwrap();
    suite
        .delegate(&members[0], bonds[2], unbonding_period3)
        .unwrap();
    suite
        .delegate(&members[2], bonds[3], unbonding_period2)
        .unwrap();
    suite
        .delegate(&members[2], bonds[4], unbonding_period3)
        .unwrap();

    assert_eq!(suite.query_balance_staking_contract().unwrap(), delegated);

    // first user unbonds on second unbonding period
    suite.unbond(&user1, 20_000u128, unbonding_period1).unwrap();
    assert_eq!(
        suite.query_staked(&user1, unbonding_period1).unwrap(),
        0u128
    );
    assert_eq!(
        suite.query_staked(&user1, unbonding_period3).unwrap(),
        10_000u128
    );

    assert_eq!(
        suite.query_rewards_power(&user1).unwrap(),
        vec![(AssetInfoValidated::Native("juno".to_string()), 6u128)]
    ); // same as before

    assert_eq!(suite.query_total_rewards_power().unwrap(), juno_power(27)); // same as before
}

#[test]
fn one_user_rebond_decrease() {
    let user = MockApi::default().addr_make("user");
    let unbonding_period1 = 1000u64;
    let unbonding_period2 = 4000u64;
    let unbonding_period3 = 8000u64;

    let mut suite = SuiteBuilder::new()
        .with_unbonding_periods(vec![
            unbonding_period1,
            unbonding_period2,
            unbonding_period3,
        ])
        .with_initial_balances(vec![(&user, 100_000)])
        .build();

    let bonds = vec![20_000u128, 30_000u128, 10_000u128];
    let delegated: u128 = bonds.iter().sum();

    suite.delegate(&user, bonds[0], unbonding_period1).unwrap();
    suite.delegate(&user, bonds[1], unbonding_period2).unwrap();
    suite.delegate(&user, bonds[2], unbonding_period3).unwrap();

    assert_eq!(suite.query_balance_staking_contract().unwrap(), delegated);

    // Rebond downwards from period 3 to 1 introducing a lockup for those tokens
    suite
        .rebond(&user, 10_000u128, unbonding_period3, unbonding_period1)
        .unwrap();

    assert_eq!(
        suite.query_staked(&user, unbonding_period1).unwrap(),
        30_000u128
    );

    assert_eq!(suite.query_staked(&user, unbonding_period3).unwrap(), 0u128);

    // Unbond 20k which is not locked. Only 10k are locked from the downwards rebond
    suite.unbond(&user, 20_000u128, unbonding_period1).unwrap();

    assert_eq!(
        suite.query_staked(&user, unbonding_period1).unwrap(),
        10_000u128
    );

    // Unbond is unsuccessful as the final 10k tokens are 'locked'
    let err = suite
        .unbond(&user, 10_000u128, unbonding_period1)
        .unwrap_err();

    assert!(err.to_string().contains("Cannot Sub with given operands"));

    // Rebond is also unsuccessful as the final 10k tokens are 'locked'
    let err = suite
        .rebond(&user, 10_000u128, unbonding_period1, unbonding_period3)
        .unwrap_err();
    assert!(err.to_string().contains("Cannot Sub with given operands"));

    // Before we advance time, ensure the locked_tokens are accounted as such in the query
    // Verify the locked and unlocked stakes via the query
    assert_eq!(
        suite.query_all_staked(&user).unwrap(),
        AllStakedResponse {
            stakes: vec![
                StakedResponse {
                    stake: Uint256::from(10_000u128),
                    total_locked: Uint256::from(10_000u128),
                    unbonding_period: 1000,
                    cw20_contract: suite.token_contract(),
                },
                StakedResponse {
                    stake: Uint256::from(30_000u128),
                    total_locked: Uint256::zero(),
                    unbonding_period: 4000,
                    cw20_contract: suite.token_contract(),
                },
                StakedResponse {
                    stake: Uint256::zero(),
                    total_locked: Uint256::zero(),
                    unbonding_period: 8000,
                    cw20_contract: suite.token_contract(),
                },
            ]
        }
    );

    // Advance time such that we can use those 10k again
    suite.update_time(unbonding_period3 - unbonding_period1 + 1);

    // Unbond is successful, the 'locked' tokens are unbonded
    suite.unbond(&user, 5_000u128, unbonding_period1).unwrap();

    // Rebond is also successful
    suite
        .rebond(&user, 5_000u128, unbonding_period1, unbonding_period3)
        .unwrap();

    // Verify again the locked and unlocked stakes via the query
    assert_eq!(
        suite.query_all_staked(&user).unwrap(),
        AllStakedResponse {
            stakes: vec![
                StakedResponse {
                    stake: Uint256::zero(),
                    total_locked: Uint256::zero(),
                    unbonding_period: 1000,
                    cw20_contract: suite.token_contract(),
                },
                StakedResponse {
                    stake: Uint256::from(30_000u128),
                    total_locked: Uint256::zero(),
                    unbonding_period: 4000,
                    cw20_contract: suite.token_contract(),
                },
                StakedResponse {
                    stake: Uint256::from(5_000u128),
                    total_locked: Uint256::zero(),
                    unbonding_period: 8000,
                    cw20_contract: suite.token_contract(),
                },
            ]
        }
    );

    let periods = suite.query_staked_periods().unwrap();
    assert_eq!(periods.len(), 3);
    assert_eq!(periods[0].unbonding_period, unbonding_period1);
    assert_eq!(
        Uint128::try_from(periods[0].total_staked).unwrap().u128(),
        0
    );
    assert_eq!(periods[1].unbonding_period, unbonding_period2);
    assert_eq!(
        Uint128::try_from(periods[1].total_staked).unwrap().u128(),
        30_000
    );
    assert_eq!(periods[2].unbonding_period, unbonding_period3);
    assert_eq!(
        Uint128::try_from(periods[2].total_staked).unwrap().u128(),
        5_000
    );
}

#[test]
fn one_user_rebond_decrease_then_rebond_again() {
    let user = MockApi::default().addr_make("user");
    let unbonding_period1 = 1000u64;
    let unbonding_period2 = 4000u64;
    let unbonding_period3 = 8000u64;
    let mut suite = SuiteBuilder::new()
        .with_unbonding_periods(vec![
            unbonding_period1,
            unbonding_period2,
            unbonding_period3,
        ])
        .with_initial_balances(vec![(&user, 100_000)])
        .build();

    let bonds = vec![20_000u128, 30_000u128, 10_000u128];
    let delegated: u128 = bonds.iter().sum();

    suite.delegate(&user, bonds[0], unbonding_period1).unwrap();
    suite.delegate(&user, bonds[1], unbonding_period2).unwrap();
    suite.delegate(&user, bonds[2], unbonding_period3).unwrap();

    assert_eq!(suite.query_balance_staking_contract().unwrap(), delegated);

    // Rebond downwards from period 3 to 1 introducing a lockup for those tokens
    suite
        .rebond(&user, 10_000u128, unbonding_period3, unbonding_period1)
        .unwrap();

    assert_eq!(
        suite.query_staked(&user, unbonding_period1).unwrap(),
        30_000u128
    );

    assert_eq!(suite.query_staked(&user, unbonding_period3).unwrap(), 0u128);

    // Unbond 20k which is not locked. Only 10k are locked from the downwards rebond
    suite.unbond(&user, 20_000u128, unbonding_period1).unwrap();

    // Unbond is unsuccessful as the final 10k tokens are 'locked'
    let err = suite
        .unbond(&user, 10_000u128, unbonding_period1)
        .unwrap_err();
    assert!(err.to_string().contains("Cannot Sub with given operands"));

    // Rebond is also unsuccessful as the final 20k tokens are 'locked'
    let err = suite
        .rebond(&user, 10_000u128, unbonding_period1, unbonding_period3)
        .unwrap_err();
    assert!(err.to_string().contains("Cannot Sub with given operands"));

    // Advance time such that we can use those 10k again
    suite.update_time(SEVEN_DAYS * 2);

    // Rebond is also successful as the final 10k tokens are no longer 'locked'
    suite
        .rebond(&user, 10_000u128, unbonding_period1, unbonding_period3)
        .unwrap();

    // Try another rebond for a smaller amount
    suite
        .rebond(&user, 5_000u128, unbonding_period3, unbonding_period1)
        .unwrap();

    // Advance time such that we can use those 5k again
    suite.update_time(SEVEN_DAYS * 2);

    // Unbond is successful, the 'locked' tokens are unbonded
    suite.unbond(&user, 5_000u128, unbonding_period1).unwrap();

    // Add more to bonding period 2
    suite
        .delegate(&user, 20_000u128, unbonding_period2)
        .unwrap();

    // Try another rebond for a large amount from another period
    suite
        .rebond(&user, 20_000u128, unbonding_period2, unbonding_period1)
        .unwrap();

    // Advance time such that we can use those 5k again
    suite.update_time(SEVEN_DAYS * 2);

    // Unbond is successful, the 'locked' tokens are unbonded
    suite.unbond(&user, 10_000u128, unbonding_period1).unwrap();

    assert_eq!(
        suite.query_staked(&user, unbonding_period1).unwrap(),
        10_000u128
    );

    // Try another rebond for a large amount for a third time
    suite
        .rebond(&user, 20_000u128, unbonding_period2, unbonding_period1)
        .unwrap();

    assert_eq!(
        suite.query_staked(&user, unbonding_period1).unwrap(),
        30_000u128
    );

    // delegate on first unbonding period
    suite
        .delegate(&user, 20_000u128, unbonding_period1)
        .unwrap();
    assert_eq!(
        suite.query_staked(&user, unbonding_period1).unwrap(),
        50_000u128
    );

    // Advance time such that we can use those 5k again
    suite.update_time(SEVEN_DAYS * 2);

    // Unbond is successful, the 'locked' tokens are unbonded
    suite.unbond(&user, 20_000u128, unbonding_period1).unwrap();
}

#[test_case(vec![1000,4000, 8000],vec![20000,30000,20000] => Some(38); "should success")]
fn query_all_staked(stake_config: Vec<UnbondingPeriod>, amount: Vec<u128>) -> Option<u64> {
    let user = MockApi::default().addr_make("user");

    let mut suite = SuiteBuilder::new()
        .with_unbonding_periods(stake_config.clone())
        .with_initial_balances(vec![(&user, 100_000)])
        .build();

    for i in 0..=(stake_config.len() - 1) {
        // delegate unbonding period
        suite.delegate(&user, amount[i], stake_config[i]).unwrap();
        // This works
        suite.query_staked(&user, stake_config[i]).unwrap();
        // This works
        suite.query_all_staked(&user).unwrap();

        assert_eq!(
            suite.query_staked(&user, stake_config[i]).unwrap(),
            amount[i]
        );
    }

    // This works
    suite.query_all_staked(&user).unwrap();

    Some(38u64)
}

#[test]
fn delegate_unbond_under_min_bond() {
    let user = MockApi::default().addr_make("user");
    let unbonding_period1 = 1000u64;
    let unbonding_period2 = 4000u64;
    let mut suite = SuiteBuilder::new()
        .with_unbonding_periods(vec![unbonding_period1, unbonding_period2])
        .with_min_bond(2_000)
        .with_initial_balances(vec![(&user, 100_000)])
        .build();

    // delegating first amount works (5_000 * 0.4 = 2_000)
    suite.delegate(&user, 5_000u128, unbonding_period1).unwrap();
    assert_eq!(
        suite.query_staked(&user, unbonding_period1).unwrap(),
        5_000u128
    );

    // delegating another amount under min bond doesn't increase voting power
    // 1_800 < 2_000
    suite.delegate(&user, 1_800u128, unbonding_period2).unwrap();
    assert_eq!(
        suite.query_staked(&user, unbonding_period2).unwrap(),
        1_800u128
    );

    // once the stake hits min_bond (2_000), count it, even if voting power (2_000 * 0.8 = 1_600) is still under min_bond
    suite.delegate(&user, 200u128, unbonding_period2).unwrap();
    assert_eq!(
        suite.query_staked(&user, unbonding_period2).unwrap(),
        2_000u128
    );

    suite.delegate(&user, 5_000u128, unbonding_period2).unwrap();
    assert_eq!(
        suite.query_staked(&user, unbonding_period2).unwrap(),
        7_000u128
    );

    // undelegate tokens from first pool so that delegation goes under min_bond
    suite.unbond(&user, 3_500u128, unbonding_period1).unwrap();
    assert_eq!(
        suite.query_staked(&user, unbonding_period1).unwrap(),
        1_500u128
    );
}
