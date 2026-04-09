use cosmwasm_std::testing::{message_info, mock_env, MockApi, MOCK_CONTRACT_ADDR};
use cosmwasm_std::{
    assert_approx_eq, attr, coins, from_json, to_json_binary, Addr, BankMsg, Binary, BlockInfo,
    Coin, CosmosMsg, Decimal256, DepsMut, Env, Fraction, ReplyOn, Response, StdError, SubMsg,
    Timestamp, Uint128, Uint256, WasmMsg,
};
use cw20::{Cw20ExecuteMsg, Cw20ReceiveMsg, MinterResponse};
use cw20_base::msg::InstantiateMsg as TokenInstantiateMsg;
use cw_multi_test::App;
use cw_utils::MsgInstantiateContractResponse;
use proptest::prelude::*;
use wyndex::asset::{
    Asset, AssetInfo, AssetInfoValidated, AssetValidated, MINIMUM_LIQUIDITY_AMOUNT,
};
use wyndex::factory::{self, PairType};
use wyndex::fee_config::FeeConfig;
use wyndex::oracle::{SamplePeriod, TwapResponse};
use wyndex::pair::{
    assert_max_spread, ContractError, Cw20HookMsg, ExecuteMsg, InstantiateMsg, PairInfo,
    PoolResponse, ReverseSimulationResponse, SimulationResponse, StakeConfig, TWAP_PRECISION,
};
use wyndex::pair::{MigrateMsg, QueryMsg};
use wyndex_test_helpers::TestAccounts;

use crate::contract::{
    accumulate_prices, compute_swap, execute, instantiate, migrate, query_pool,
    query_reverse_simulation, query_share, query_simulation,
};
use crate::contract::{compute_offer_amount, query};
use crate::state::{Config, CONFIG};
// TODO: Copied here just as a temporary measure
use crate::mock_querier::mock_dependencies;

fn store_liquidity_token(deps: DepsMut, contract_addr: String) {
    let res = MsgInstantiateContractResponse {
        contract_address: contract_addr,
        data: None,
    };

    let mut config = CONFIG.load(deps.storage).unwrap();
    let _res = wyndex::pair::instantiate_lp_token_reply(
        &deps,
        res,
        &config.factory_addr,
        &mut config.pair_info,
    )
    .unwrap();
    CONFIG.save(deps.storage, &config).unwrap();
}

fn default_stake_config() -> StakeConfig {
    StakeConfig {
        staking_code_id: 11,
        tokens_per_power: Uint256::new(1000),
        min_bond: Uint256::new(1000),
        unbonding_periods: vec![60 * 60 * 24 * 7],
        max_distributions: 6,
        converter: None,
    }
}

#[test]
fn proper_initialization() {
    let mut deps = mock_dependencies(&[]);
    let a = TestAccounts::new(&deps.api);
    let liquidity0000 = a.trader;
    let addr0000 = a.whale;
    let asset0000 = a.owner;
    let factory = a.beneficiary;
    let sender = addr0000;

    deps.querier.with_token_balances(&[(
        &asset0000.to_string(),
        &[(&MOCK_CONTRACT_ADDR.to_string(), &Uint128::new(123u128))],
    )]);

    let msg = InstantiateMsg {
        factory_addr: factory.to_string(),
        asset_infos: vec![
            AssetInfo::Native("uusd".to_string()),
            AssetInfo::Token(asset0000.to_string()),
        ],
        token_code_id: 10u64,
        init_params: None,
        staking_config: default_stake_config(),
        trading_starts: 0,
        fee_config: FeeConfig {
            total_fee_bps: 0,
            protocol_fee_bps: 0,
        },
        circuit_breaker: None,
    };

    // We can just call .unwrap() to assert this was a success
    let env = mock_env();
    let info = message_info(&sender, &[]);
    let res = instantiate(deps.as_mut(), env, info, msg).unwrap();
    assert_eq!(
        res.messages,
        vec![SubMsg {
            msg: WasmMsg::Instantiate {
                code_id: 10u64,
                msg: to_json_binary(&TokenInstantiateMsg {
                    name: "UUSD-MAPP-LP".to_string(),
                    symbol: "uLP".to_string(),
                    decimals: 6,
                    initial_balances: vec![],
                    mint: Some(MinterResponse {
                        minter: MOCK_CONTRACT_ADDR.to_string(),
                        cap: None,
                    }),
                    marketing: None
                })
                .unwrap(),
                funds: vec![],
                admin: Some(MockApi::default().addr_make("owner").to_string()),
                label: String::from("Wyndex LP token"),
            }
            .into(),
            id: 1,
            gas_limit: None,
            reply_on: ReplyOn::Success,
            payload: Binary::new(vec![])
        },]
    );

    // Store liquidity token
    store_liquidity_token(deps.as_mut(), liquidity0000.to_string());

    // It worked, let's query the state
    let pair_info = CONFIG.load(deps.as_ref().storage).unwrap().pair_info;
    assert_eq!(liquidity0000, pair_info.liquidity_token);
    assert_eq!(
        pair_info.asset_infos,
        [
            AssetInfoValidated::Native("uusd".to_string()),
            AssetInfoValidated::Token(asset0000)
        ]
    );
}

// Rather long test the does a few things
// First for sanity, does a provide liquidity
// Then through migration marks the contract as frozen and assigns addr0000 as the circuit_breaker, the one who can unfreeze the contract and refreeze via an ExecuteMsg
// Then we try to provide liquidity again, which should fail
// We also try a native swap, a cw20 swap and an UpdateFees, all fails with ContractFrozen
// However, withdraw liquidity is not frozen and people can still withdraw
// We then try to unfreeze with addr0001, which should fail
// We then try to unfreeze with addr0000, which should succeed and to prove this we try to
// provide liquidity again and swap, which should both succeed
#[test]
fn test_freezing_a_pool_blocking_actions_then_unfreeze() {
    let mut deps = mock_dependencies(&[Coin {
        denom: "uusd".to_string(),
        amount: Uint256::new(200_000000000000000000u128),
    }]);
    let offer_amount = Uint256::new(1500000000u128);

    let a = TestAccounts::new(&deps.api);
    let liquidity0000 = a.trader;
    let addr0000 = a.whale;
    let asset0000 = a.owner;
    let addr0001 = a.user;
    let factory = a.beneficiary;
    let sender = addr0000.clone();

    deps.querier.with_token_balances(&[
        (
            &asset0000.to_string(),
            &[(&MOCK_CONTRACT_ADDR.to_string(), &Uint128::new(0))],
        ),
        (
            &liquidity0000.to_string(),
            &[(&MOCK_CONTRACT_ADDR.to_string(), &Uint128::new(0))],
        ),
    ]);

    let msg = InstantiateMsg {
        asset_infos: vec![
            AssetInfo::Native("uusd".to_string()),
            AssetInfo::Token(asset0000.to_string()),
        ],
        token_code_id: 10u64,
        factory_addr: factory.to_string(),
        init_params: None,
        staking_config: default_stake_config(),
        trading_starts: 0,
        fee_config: FeeConfig {
            total_fee_bps: 0,
            protocol_fee_bps: 0,
        },
        circuit_breaker: None,
    };

    let env = mock_env();
    let info = message_info(&addr0000, &[]);
    // We can just call .unwrap() to assert this was a success
    let _res = instantiate(deps.as_mut(), env, info, msg).unwrap();

    // Store liquidity token
    store_liquidity_token(deps.as_mut(), liquidity0000.to_string());

    // Successfully provide liquidity for the existing pool
    let msg = ExecuteMsg::ProvideLiquidity {
        assets: vec![
            Asset {
                info: AssetInfo::Token(asset0000.to_string()),
                amount: Uint256::from(100_000000000000000000u128),
            },
            Asset {
                info: AssetInfo::Native("uusd".to_string()),
                amount: Uint256::from(100_000000000000000000u128),
            },
        ],
        slippage_tolerance: None,
        receiver: None,
    };

    let env = mock_env();
    let info = message_info(
        &addr0000,
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint256::from(100_000000000000000000u128),
        }],
    );
    // Do one successful action before freezing just for sanity
    execute(deps.as_mut(), env.clone(), info, msg).unwrap();

    // Manually set the correct balances for the pool
    deps.querier.with_balance(&[(
        &MOCK_CONTRACT_ADDR.to_string(),
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint256::new(100_000000000000000000),
        }],
    )]);
    deps.querier.with_token_balances(&[
        (
            &liquidity0000.to_string(),
            &[
                (
                    &MOCK_CONTRACT_ADDR.to_string(),
                    &Uint128::try_from(MINIMUM_LIQUIDITY_AMOUNT).unwrap(),
                ),
                (
                    &addr0000.to_string(),
                    &Uint128::try_from(
                        Uint256::new(100_000000000000000000) - MINIMUM_LIQUIDITY_AMOUNT,
                    )
                    .unwrap(),
                ),
            ],
        ),
        (
            &asset0000.to_string(),
            &[(
                &MOCK_CONTRACT_ADDR.to_string(),
                &Uint128::new(100_000000000000000000),
            )],
        ),
    ]);

    // Migrate with the freeze migrate message
    migrate(
        deps.as_mut(),
        env.clone(),
        MigrateMsg::UpdateFreeze {
            frozen: true,
            circuit_breaker: Some(addr0000.to_string()),
        },
    )
    .unwrap();

    // Failing Execute Actions due to frozen

    // This should now fail, its a good TX with all the normal setup done but because of freezing it should fail
    let msg = ExecuteMsg::ProvideLiquidity {
        assets: vec![
            Asset {
                info: AssetInfo::Token(asset0000.to_string()),
                amount: Uint256::from(100_000000000000000000u128),
            },
            Asset {
                info: AssetInfo::Native("uusd".to_string()),
                amount: Uint256::from(200_000000000000000000u128),
            },
        ],
        slippage_tolerance: Some(Decimal256::percent(50)),
        receiver: None,
    };

    let env = mock_env_with_block_time(env.block.time.seconds() + 1000);
    let info = message_info(
        &addr0000,
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint256::from(200_000000000000000000u128),
        }],
    );

    // Assert an error and that its frozen
    let res: ContractError = execute(deps.as_mut(), env.clone(), info, msg).unwrap_err();
    assert_eq!(res, ContractError::ContractFrozen {});
    // Also do a swap, which should also fail
    let msg = ExecuteMsg::Swap {
        offer_asset: Asset {
            info: AssetInfo::Native("uusd".to_string()),
            amount: 1_000u128.into(),
        },
        to: None,
        max_spread: None,
        belief_price: None,
        ask_asset_info: None,
        referral_address: None,
        referral_commission: None,
    };

    let info = message_info(
        &addr0000,
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint256::from(1000u128),
        }],
    );
    // Assert an error and that its frozen
    let res: ContractError = execute(deps.as_mut(), env.clone(), info.clone(), msg).unwrap_err();
    assert_eq!(res, ContractError::ContractFrozen {});

    let msg = ExecuteMsg::UpdateFees {
        fee_config: FeeConfig {
            total_fee_bps: 5,
            protocol_fee_bps: 5,
        },
    };
    let res = execute(deps.as_mut(), env.clone(), info, msg).unwrap_err();
    assert_eq!(res, ContractError::ContractFrozen {});

    // Normal sell but with CW20
    let msg = ExecuteMsg::Receive(Cw20ReceiveMsg {
        sender: addr0000.to_string(),
        amount: offer_amount,
        msg: to_json_binary(&Cw20HookMsg::Swap {
            ask_asset_info: None,
            belief_price: None,
            max_spread: Some(Decimal256::percent(50)),
            to: None,
            referral_address: None,
            referral_commission: None,
        })
        .unwrap(),
    });
    let info = message_info(&asset0000, &[]);

    let res = execute(deps.as_mut(), env.clone(), info, msg).unwrap_err();
    assert_eq!(res, ContractError::ContractFrozen {});

    // But we can withdraw liquidity

    // Withdraw liquidity
    let msg = ExecuteMsg::Receive(Cw20ReceiveMsg {
        sender: addr0000.to_string(),
        msg: to_json_binary(&Cw20HookMsg::WithdrawLiquidity { assets: vec![] }).unwrap(),
        amount: Uint256::new(100u128),
    });

    let info = message_info(&liquidity0000, &[]);
    // We just want to ensure it doesn't fail with a ContractFrozen error
    execute(deps.as_mut(), env.clone(), info, msg).unwrap();

    // Unfreeze the pool again using the Freeze message rather than another migrate
    let msg = ExecuteMsg::Freeze { frozen: false };
    // First try a failing case with addr0001
    let info = message_info(&addr0001, &[]);
    // Rather than being unfrozen it returns unauthorized as addr0000 is the only addr that can currently call Freeze unless another migration changes that
    let err = execute(deps.as_mut(), env.clone(), info, msg.clone()).unwrap_err();
    assert_eq!(err, ContractError::Unauthorized {});
    // But the assigned circuit_breaker address can do an unfreeze with the ExecuteMsg variant
    let info = message_info(&addr0000, &[]);
    // And it works
    execute(deps.as_mut(), env.clone(), info, msg).unwrap();

    // Testing actions working again after unfreeze

    // Initialize token balance to 1:1
    deps.querier.with_balance(&[(
        &MOCK_CONTRACT_ADDR.to_string(),
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint256::new(100_000000000000000000 + 99_000000000000000000 /* user deposit must be pre-applied */),
        }],
    )]);

    deps.querier.with_token_balances(&[
        (
            &liquidity0000.to_string(),
            &[(
                &MOCK_CONTRACT_ADDR.to_string(),
                &Uint128::new(100_000000000000000000),
            )],
        ),
        (
            &asset0000.to_string(),
            &[(
                &MOCK_CONTRACT_ADDR.to_string(),
                &Uint128::new(100_000000000000000000),
            )],
        ),
    ]);

    // Successfully provides liquidity
    let msg = ExecuteMsg::ProvideLiquidity {
        assets: vec![
            Asset {
                info: AssetInfo::Token(asset0000.to_string()),
                amount: Uint256::from(100_000000000000000000u128),
            },
            Asset {
                info: AssetInfo::Native("uusd".to_string()),
                amount: Uint256::from(99_000000000000000000u128),
            },
        ],
        slippage_tolerance: Some(Decimal256::percent(1)),
        receiver: None,
    };

    let info = message_info(
        &addr0001,
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint256::from(99_000000000000000000u128),
        }],
    );
    execute(deps.as_mut(), env.clone(), info, msg).unwrap();

    // Normal sell but with CW20
    let msg = ExecuteMsg::Receive(Cw20ReceiveMsg {
        sender: addr0000.to_string(),
        amount: offer_amount,
        msg: to_json_binary(&Cw20HookMsg::Swap {
            ask_asset_info: None,
            belief_price: None,
            max_spread: Some(Decimal256::percent(50)),
            to: None,
            referral_address: None,
            referral_commission: None,
        })
        .unwrap(),
    });
    let info = message_info(&asset0000, &[]);

    execute(deps.as_mut(), env, info, msg).unwrap();
}

fn default_addrs() -> Vec<Addr> {
    vec![
        MockApi::default().addr_make("owner"),
        MockApi::default().addr_make("factory"),
        MockApi::default().addr_make("addr0000"),
        MockApi::default().addr_make("addr0001"),
        Addr::unchecked(MOCK_CONTRACT_ADDR),
    ]
}

#[test]
fn provide_liquidity() {
    let _d = default_addrs();

    let mut deps = mock_dependencies(&[Coin {
        denom: "uusd".to_string(),
        amount: Uint256::new(200_000000000000000000u128),
    }]);

    let a = TestAccounts::new(&deps.api);
    let liquidity0000 = a.trader;
    let addr0000 = a.whale;
    let addr0001 = a.user;
    let asset0000 = a.owner;
    let factory = a.beneficiary;

    deps.querier.with_token_balances(&[
        (
            &asset0000.to_string(),
            &[(&MOCK_CONTRACT_ADDR.to_string(), &Uint128::new(0))],
        ),
        (
            &liquidity0000.to_string(),
            &[(&MOCK_CONTRACT_ADDR.to_string(), &Uint128::new(0))],
        ),
    ]);

    let msg = InstantiateMsg {
        asset_infos: vec![
            AssetInfo::Native("uusd".to_string()),
            AssetInfo::Token(asset0000.to_string()),
        ],
        token_code_id: 10u64,
        factory_addr: factory.to_string(),
        init_params: None,
        staking_config: default_stake_config(),
        trading_starts: 0,
        fee_config: FeeConfig {
            total_fee_bps: 0,
            protocol_fee_bps: 0,
        },
        circuit_breaker: None,
    };

    let env = mock_env();
    let info = message_info(&addr0000, &[]);
    // We can just call .unwrap() to assert this was a success
    let _res = instantiate(deps.as_mut(), env, info, msg).unwrap();

    // Store liquidity token
    store_liquidity_token(deps.as_mut(), liquidity0000.to_string());

    // Successfully provide liquidity for the existing pool
    let msg = ExecuteMsg::ProvideLiquidity {
        assets: vec![
            Asset {
                info: AssetInfo::Token(asset0000.to_string()),
                amount: Uint256::from(100_000000000000000000u128),
            },
            Asset {
                info: AssetInfo::Native("uusd".to_string()),
                amount: Uint256::from(100_000000000000000000u128),
            },
        ],
        slippage_tolerance: None,
        receiver: None,
    };

    let env = mock_env();
    let info = message_info(
        &addr0000,
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint256::from(100_000000000000000000u128),
        }],
    );
    let res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();
    let transfer_from_msg = res.messages.get(0).expect("no message");
    let mint_min_liquidity_msg = res.messages.get(1).expect("no message");
    let mint_receiver_msg = res.messages.get(2).expect("no message");
    assert_eq!(
        transfer_from_msg,
        &SubMsg {
            msg: WasmMsg::Execute {
                contract_addr: asset0000.to_string(),
                msg: to_json_binary(&Cw20ExecuteMsg::TransferFrom {
                    owner: addr0000.to_string(),
                    recipient: MOCK_CONTRACT_ADDR.to_string(),
                    amount: Uint256::from(100_000000000000000000u128),
                })
                .unwrap(),
                funds: vec![],
            }
            .into(),
            id: 0,
            gas_limit: None,
            reply_on: ReplyOn::Never,
            payload: Binary::new(vec![])
        }
    );
    assert_eq!(
        mint_min_liquidity_msg,
        &SubMsg {
            msg: WasmMsg::Execute {
                contract_addr: liquidity0000.to_string(),
                msg: to_json_binary(&Cw20ExecuteMsg::Mint {
                    recipient: MOCK_CONTRACT_ADDR.to_string(),
                    amount: Uint256::from(1000_u128),
                })
                .unwrap(),
                funds: vec![],
            }
            .into(),
            id: 0,
            gas_limit: None,
            reply_on: ReplyOn::Never,
            payload: Binary::new(vec![])
        }
    );
    assert_eq!(
        mint_receiver_msg,
        &SubMsg {
            msg: WasmMsg::Execute {
                contract_addr: liquidity0000.to_string(),
                msg: to_json_binary(&Cw20ExecuteMsg::Mint {
                    recipient: addr0000.to_string(),
                    amount: Uint256::from(99_999999999999999000u128),
                })
                .unwrap(),
                funds: vec![],
            }
            .into(),
            id: 0,
            gas_limit: None,
            reply_on: ReplyOn::Never,
            payload: Binary::new(vec![])
        }
    );

    // Provide more liquidity 1:2, which is not propotional to 1:1,
    // It must accept 1:1 and treat the leftover amount as a donation
    deps.querier.with_balance(&[(
        &MOCK_CONTRACT_ADDR.to_string(),
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint256::new(200_000000000000000000 + 200_000000000000000000 /* user deposit must be pre-applied */),
        }],
    )]);

    deps.querier.with_token_balances(&[
        (
            &liquidity0000.to_string(),
            &[(
                &MOCK_CONTRACT_ADDR.to_string(),
                &Uint128::new(100_000000000000000000),
            )],
        ),
        (
            &asset0000.to_string(),
            &[(
                &MOCK_CONTRACT_ADDR.to_string(),
                &Uint128::new(200_000000000000000000),
            )],
        ),
    ]);

    let msg = ExecuteMsg::ProvideLiquidity {
        assets: vec![
            Asset {
                info: AssetInfo::Token(asset0000.to_string()),
                amount: Uint256::from(100_000000000000000000u128),
            },
            Asset {
                info: AssetInfo::Native("uusd".to_string()),
                amount: Uint256::from(200_000000000000000000u128),
            },
        ],
        slippage_tolerance: Some(Decimal256::percent(50)),
        receiver: None,
    };

    let env = mock_env_with_block_time(env.block.time.seconds() + 1000);
    let info = message_info(
        &addr0000,
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint256::from(200_000000000000000000u128),
        }],
    );

    // Only accept 100, then 50 share will be generated with 100 * (100 / 200)
    let res: Response = execute(deps.as_mut(), env, info, msg).unwrap();
    let transfer_from_msg = res.messages.get(0).expect("no message");
    let mint_msg = res.messages.get(1).expect("no message");
    assert_eq!(
        transfer_from_msg,
        &SubMsg {
            msg: WasmMsg::Execute {
                contract_addr: asset0000.to_string(),
                msg: to_json_binary(&Cw20ExecuteMsg::TransferFrom {
                    owner: addr0000.to_string(),
                    recipient: MOCK_CONTRACT_ADDR.to_string(),
                    amount: Uint256::from(100_000000000000000000u128),
                })
                .unwrap(),
                funds: vec![],
            }
            .into(),
            id: 0,
            gas_limit: None,
            reply_on: ReplyOn::Never,
            payload: Binary::new(vec![])
        }
    );
    assert_eq!(
        mint_msg,
        &SubMsg {
            msg: WasmMsg::Execute {
                contract_addr: liquidity0000.to_string(),
                msg: to_json_binary(&Cw20ExecuteMsg::Mint {
                    recipient: addr0000.to_string(),
                    amount: Uint256::from(50_000000000000000000u128),
                })
                .unwrap(),
                funds: vec![],
            }
            .into(),
            id: 0,
            gas_limit: None,
            reply_on: ReplyOn::Never,
            payload: Binary::new(vec![])
        }
    );

    // Check wrong argument
    let msg = ExecuteMsg::ProvideLiquidity {
        assets: vec![
            Asset {
                info: AssetInfo::Token(asset0000.to_string()),
                amount: Uint256::from(100_000000000000000000u128),
            },
            Asset {
                info: AssetInfo::Native("uusd".to_string()),
                amount: Uint256::from(50_000000000000000000u128),
            },
        ],
        slippage_tolerance: None,
        receiver: None,
    };

    let env = mock_env();
    let info = message_info(
        &addr0000,
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint256::from(100_000000000000000000u128),
        }],
    );
    let res = execute(deps.as_mut(), env.clone(), info, msg).unwrap_err();
    assert!(res.to_string().contains("Native token balance mismatch between the argument and the transferred"));

    // Initialize token amount to the 1:1 ratio
    deps.querier.with_balance(&[(
        &MOCK_CONTRACT_ADDR.to_string(),
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint256::new(100_000000000000000000 + 100_000000000000000000 /* user deposit must be pre-applied */),
        }],
    )]);

    deps.querier.with_token_balances(&[
        (
            &liquidity0000.to_string(),
            &[(
                &MOCK_CONTRACT_ADDR.to_string(),
                &Uint128::new(100_000000000000000000),
            )],
        ),
        (
            &asset0000.to_string(),
            &[(
                &MOCK_CONTRACT_ADDR.to_string(),
                &Uint128::new(100_000000000000000000),
            )],
        ),
    ]);

    // Failed because the price is under slippage_tolerance
    let msg = ExecuteMsg::ProvideLiquidity {
        assets: vec![
            Asset {
                info: AssetInfo::Token(asset0000.to_string()),
                amount: Uint256::from(98_000000000000000000u128),
            },
            Asset {
                info: AssetInfo::Native("uusd".to_string()),
                amount: Uint256::from(100_000000000000000000u128),
            },
        ],
        slippage_tolerance: Some(Decimal256::percent(1)),
        receiver: None,
    };

    let env = mock_env_with_block_time(env.block.time.seconds() + 1000);
    let info = message_info(
        &addr0001,
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint256::from(100_000000000000000000u128),
        }],
    );
    let res = execute(deps.as_mut(), env.clone(), info, msg).unwrap_err();
    assert_eq!(res, ContractError::MaxSlippageAssertion {});

    // Initialize token balance to 1:1
    deps.querier.with_balance(&[(
        &MOCK_CONTRACT_ADDR.to_string(),
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint256::new(100_000000000000000000 + 98_000000000000000000 /* user deposit must be pre-applied */),
        }],
    )]);

    // Failed because the price is under slippage_tolerance
    let msg = ExecuteMsg::ProvideLiquidity {
        assets: vec![
            Asset {
                info: AssetInfo::Token(asset0000.to_string()),
                amount: Uint256::from(100_000000000000000000u128),
            },
            Asset {
                info: AssetInfo::Native("uusd".to_string()),
                amount: Uint256::from(98_000000000000000000u128),
            },
        ],
        slippage_tolerance: Some(Decimal256::percent(1)),
        receiver: None,
    };

    let env = mock_env_with_block_time(env.block.time.seconds() + 1000);
    let info = message_info(
        &addr0001,
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint256::from(98_000000000000000000u128),
        }],
    );
    let res = execute(deps.as_mut(), env.clone(), info, msg).unwrap_err();
    assert_eq!(res, ContractError::MaxSlippageAssertion {});

    // Initialize token amount with a 1:1 ratio
    deps.querier.with_balance(&[(
        &MOCK_CONTRACT_ADDR.to_string(),
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint256::new(100_000000000000000000 + 100_000000000000000000 /* user deposit must be pre-applied */),
        }],
    )]);

    // Successfully provides liquidity
    let msg = ExecuteMsg::ProvideLiquidity {
        assets: vec![
            Asset {
                info: AssetInfo::Token(asset0000.to_string()),
                amount: Uint256::from(99_000000000000000000u128),
            },
            Asset {
                info: AssetInfo::Native("uusd".to_string()),
                amount: Uint256::from(100_000000000000000000u128),
            },
        ],
        slippage_tolerance: Some(Decimal256::percent(1)),
        receiver: None,
    };

    let env = mock_env_with_block_time(env.block.time.seconds() + 1000);
    let info = message_info(
        &addr0001,
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint256::from(100_000000000000000000u128),
        }],
    );
    let _res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();

    // Initialize token balance to 1:1
    deps.querier.with_balance(&[(
        &MOCK_CONTRACT_ADDR.to_string(),
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint256::new(100_000000000000000000 + 99_000000000000000000 /* user deposit must be pre-applied */),
        }],
    )]);

    // Successfully provides liquidity
    let msg = ExecuteMsg::ProvideLiquidity {
        assets: vec![
            Asset {
                info: AssetInfo::Token(asset0000.to_string()),
                amount: Uint256::from(100_000000000000000000u128),
            },
            Asset {
                info: AssetInfo::Native("uusd".to_string()),
                amount: Uint256::from(99_000000000000000000u128),
            },
        ],
        slippage_tolerance: Some(Decimal256::percent(1)),
        receiver: None,
    };

    let env = mock_env_with_block_time(env.block.time.seconds() + 1000);
    let info = message_info(
        &addr0001,
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint256::from(99_000000000000000000u128),
        }],
    );
    execute(deps.as_mut(), env, info, msg).unwrap();

    let msg = ExecuteMsg::ProvideLiquidity {
        assets: vec![
            Asset {
                info: AssetInfo::Token(asset0000.to_string()),
                amount: Uint256::zero(),
            },
            Asset {
                info: AssetInfo::Native("uusd".to_string()),
                amount: Uint256::from(99_000000000000000000u128),
            },
        ],
        slippage_tolerance: Some(Decimal256::percent(1)),
        receiver: None,
    };
    let info = message_info(
        &addr0001,
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint256::from(99_000000000000000000u128),
        }],
    );
    let err = execute(deps.as_mut(), mock_env(), info, msg).unwrap_err();
    assert_eq!(err, ContractError::InvalidZeroAmount {});

    let msg = ExecuteMsg::ProvideLiquidity {
        assets: vec![
            Asset {
                info: AssetInfo::Token(asset0000.to_string()),
                amount: Uint256::from(100_000000000000000000u128),
            },
            Asset {
                info: AssetInfo::Native("uusd".to_string()),
                amount: Uint256::from(100_000000000000000000u128),
            },
        ],
        slippage_tolerance: Some(Decimal256::percent(51)),
        receiver: None,
    };
    let info = message_info(
        &addr0001,
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint256::from(100_000000000000000000u128),
        }],
    );
    let err = execute(deps.as_mut(), mock_env(), info, msg).unwrap_err();
    assert_eq!(err, ContractError::AllowedSpreadAssertion {});
}

#[test]
fn withdraw_liquidity() {
    let mut deps = mock_dependencies(&[Coin {
        denom: "uusd".to_string(),
        amount: Uint256::new(100u128),
    }]);
    let a = TestAccounts::new(&deps.api);
    let liquidity0000 = a.trader;
    let addr0000 = a.whale;
    let asset0000 = a.owner;
    let factory = a.beneficiary;

    deps.querier.with_token_balances(&[
        (
            &liquidity0000.to_string(),
            &[
                (&addr0000.to_string(), &Uint128::new(100u128)),
                (&MOCK_CONTRACT_ADDR.to_string(), &Uint128::new(1000u128)), // MIN_LIQUIDITY_AMOUNT
            ],
        ),
        (
            &asset0000.to_string(),
            &[(&MOCK_CONTRACT_ADDR.to_string(), &Uint128::new(100u128))],
        ),
    ]);

    let msg = InstantiateMsg {
        asset_infos: vec![
            AssetInfo::Native("uusd".to_string()),
            AssetInfo::Token(asset0000.to_string()),
        ],
        token_code_id: 10u64,

        factory_addr: factory.to_string(),
        init_params: None,
        staking_config: default_stake_config(),
        trading_starts: 0,
        fee_config: FeeConfig {
            total_fee_bps: 0,
            protocol_fee_bps: 0,
        },
        circuit_breaker: None,
    };

    let env = mock_env();
    let info = message_info(&addr0000, &[]);
    // We can just call .unwrap() to assert this was a success
    let _res = instantiate(deps.as_mut(), env, info, msg).unwrap();

    // Store liquidity token
    store_liquidity_token(deps.as_mut(), liquidity0000.to_string());

    // need to initialize oracle, because we don't call `provide_liquidity` in this test
    wyndex::oracle::initialize_oracle(
        &mut deps.storage,
        &mock_env_with_block_time(0),
        Decimal256::one(),
    )
    .unwrap();

    // Withdraw liquidity
    let msg = ExecuteMsg::Receive(Cw20ReceiveMsg {
        sender: addr0000.to_string(),
        msg: to_json_binary(&Cw20HookMsg::WithdrawLiquidity { assets: vec![] }).unwrap(),
        amount: Uint256::new(100u128),
    });

    let env = mock_env();
    let info = message_info(&liquidity0000, &[]);
    let res = execute(deps.as_mut(), env, info, msg).unwrap();
    let log_withdrawn_share = res.attributes.get(2).expect("no log");
    let log_refund_assets = res.attributes.get(3).expect("no log");
    let msg_refund_0 = res.messages.get(0).expect("no message");
    let msg_refund_1 = res.messages.get(1).expect("no message");
    let msg_burn_liquidity = res.messages.get(2).expect("no message");
    assert_eq!(
        msg_refund_0,
        &SubMsg {
            msg: CosmosMsg::Bank(BankMsg::Send {
                to_address: addr0000.to_string(),
                amount: vec![Coin {
                    denom: "uusd".to_string(),
                    amount: Uint256::from(9u128),
                }],
            }),
            id: 0,
            gas_limit: None,
            reply_on: ReplyOn::Never,
            payload: Binary::new(vec![])
        }
    );
    assert_eq!(
        msg_refund_1,
        &SubMsg {
            msg: WasmMsg::Execute {
                contract_addr: asset0000.to_string(),
                msg: to_json_binary(&Cw20ExecuteMsg::Transfer {
                    recipient: addr0000.to_string(),
                    amount: Uint256::from(9u128),
                })
                .unwrap(),
                funds: vec![],
            }
            .into(),
            id: 0,
            gas_limit: None,
            reply_on: ReplyOn::Never,
            payload: Binary::new(vec![])
        }
    );
    assert_eq!(
        msg_burn_liquidity,
        &SubMsg {
            msg: WasmMsg::Execute {
                contract_addr: liquidity0000.to_string(),
                msg: to_json_binary(&Cw20ExecuteMsg::Burn {
                    amount: Uint256::from(100u128),
                })
                .unwrap(),
                funds: vec![],
            }
            .into(),
            id: 0,
            gas_limit: None,
            reply_on: ReplyOn::Never,
            payload: Binary::new(vec![])
        }
    );

    assert_eq!(
        log_withdrawn_share,
        &attr("withdrawn_share", 100u128.to_string())
    );
    assert_eq!(
        log_refund_assets,
        &attr("refund_assets", format!("9uusd, 9{}", MockApi::default().addr_make("owner")))
    );
}

#[test]
fn query_twap() {
    let mut deps = mock_dependencies(&[]);
    let mut env = mock_env();

    let a = TestAccounts::new(&deps.api);
    let liquidity0000 = a.trader;
    let addr0000 = a.whale;
    let asset0000 = a.fee_receiver;
    let factory = a.beneficiary;
    let user = a.user;
    let owner = a.owner;

    // setup some cw20 tokens, so the queries don't fail
    deps.querier.with_token_balances(&[
        (
            &asset0000.to_string(),
            &[(&MOCK_CONTRACT_ADDR.into(), &0u128.into())],
        ),
        (
            &liquidity0000.to_string(),
            &[(&MOCK_CONTRACT_ADDR.into(), &0u128.into())],
        ),
    ]);

    let uusd = AssetInfoValidated::Native("uusd".to_string());
    let token = AssetInfoValidated::Token(asset0000.clone());

    // instantiate the contract
    let msg = InstantiateMsg {
        asset_infos: vec![uusd.clone().into(), token.clone().into()],
        token_code_id: 10u64,
        factory_addr: factory.to_string(),
        init_params: None,
        staking_config: default_stake_config(),
        trading_starts: 0,
        fee_config: FeeConfig {
            total_fee_bps: 0,
            protocol_fee_bps: 0,
        },
        circuit_breaker: None,
    };
    instantiate(deps.as_mut(), env.clone(), message_info(&owner, &[]), msg).unwrap();

    // Store the liquidity token
    store_liquidity_token(deps.as_mut(), liquidity0000.to_string());

    // provide liquidity to get a first price
    let msg = ExecuteMsg::ProvideLiquidity {
        assets: vec![
            Asset {
                info: uusd.clone().into(),
                amount: 1_000_000u128.into(),
            },
            Asset {
                info: token.into(),
                amount: 1_000_000u128.into(),
            },
        ],
        slippage_tolerance: None,
        receiver: None,
    };
    // need to set balance manually to simulate funds being sent
    deps.querier
        .with_balance(&[(&MOCK_CONTRACT_ADDR.into(), &coins(1_000_000u128, "uusd"))]);
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&user, &coins(1_000_000u128, "uusd")),
        msg,
    )
    .unwrap();

    // set cw20 balance manually
    deps.querier.with_token_balances(&[
        (
            &asset0000.to_string(),
            &[(&MOCK_CONTRACT_ADDR.into(), &1_000_000u128.into())],
        ),
        (
            &liquidity0000.to_string(),
            &[(&MOCK_CONTRACT_ADDR.into(), &0u128.into())],
        ),
    ]);

    // querying TWAP after first price change should fail, because only one price is recorded
    let err = query(
        deps.as_ref(),
        env.clone(),
        QueryMsg::Twap {
            duration: SamplePeriod::HalfHour,
            start_age: 1,
            end_age: Some(0),
        },
    )
    .unwrap_err();

    assert_eq!(
        err.to_string(),
        StdError::msg("start index is earlier than earliest recorded price data").to_string()
    );

    // forward time half an hour
    const HALF_HOUR: u64 = 30 * 60;
    env.block.time = env.block.time.plus_seconds(HALF_HOUR);

    // swap to get a second price
    let msg = ExecuteMsg::Swap {
        offer_asset: Asset {
            info: uusd.into(),
            amount: 1_000u128.into(),
        },
        to: None,
        max_spread: None,
        belief_price: None,
        ask_asset_info: None,
        referral_address: None,
        referral_commission: None,
    };
    // need to set balance manually to simulate funds being sent
    deps.querier
        .with_balance(&[(&MOCK_CONTRACT_ADDR.into(), &coins(1_001_000u128, "uusd"))]);
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&user, &coins(1_000u128, "uusd")),
        msg,
    )
    .unwrap();

    // forward time half an hour again for the last change to accumulate
    env.block.time = env.block.time.plus_seconds(HALF_HOUR);

    // query twap after swap price change
    let twap: TwapResponse = from_json(
        &query(
            deps.as_ref(),
            env,
            QueryMsg::Twap {
                duration: SamplePeriod::HalfHour,
                start_age: 1,
                end_age: Some(0),
            },
        )
        .unwrap(),
    )
    .unwrap();

    assert!(twap.a_per_b > Decimal256::one());
    assert!(twap.b_per_a < Decimal256::one());
    assert_approx_eq!(
        Uint128::try_from(twap.a_per_b.numerator()).unwrap(),
        Uint128::try_from(Decimal256::from_ratio(1_001_000u128, 999_000u128).numerator()).unwrap(),
        "0.000002",
        "twap should be slightly below 1"
    );
    assert_approx_eq!(
        Uint128::try_from(twap.b_per_a.numerator()).unwrap(),
        Uint128::try_from(Decimal256::from_ratio(999_000u128, 1_001_000u128).numerator()).unwrap(),
        "0.000002",
        "twap should be slightly above 1"
    );
}

#[test]
fn try_native_to_token() {
    let total_share = Uint128::new(30000000000u128);
    let asset_pool_amount = Uint128::new(20000000000u128);
    let collateral_pool_amount = Uint256::new(30000000000u128);
    let offer_amount = Uint256::new(1500000000u128);

    let mut deps = mock_dependencies(&[Coin {
        denom: "uusd".to_string(),
        amount: collateral_pool_amount + offer_amount, /* user deposit must be pre-applied */
    }]);

    let a = TestAccounts::new(&deps.api);
    let liquidity0000 = a.trader;
    let addr0000 = a.whale;
    let asset0000 = a.owner;
    let factory = a.beneficiary;

    deps.querier.with_token_balances(&[
        (
            &liquidity0000.to_string(),
            &[(&MOCK_CONTRACT_ADDR.to_string(), &total_share)],
        ),
        (
            &asset0000.to_string(),
            &[(&MOCK_CONTRACT_ADDR.to_string(), &asset_pool_amount)],
        ),
    ]);

    let msg = InstantiateMsg {
        asset_infos: vec![
            AssetInfo::Native("uusd".to_string()),
            AssetInfo::Token(asset0000.to_string()),
        ],
        token_code_id: 10u64,
        factory_addr: factory.to_string(),
        init_params: None,
        staking_config: default_stake_config(),
        trading_starts: 0,
        fee_config: FeeConfig {
            total_fee_bps: 30,
            protocol_fee_bps: 1660,
        },
        circuit_breaker: None,
    };

    let env = mock_env();
    let info = message_info(&addr0000, &[]);
    // We can just call .unwrap() to assert this was a success
    let _res = instantiate(deps.as_mut(), env, info, msg).unwrap();

    // Store liquidity token
    store_liquidity_token(deps.as_mut(), liquidity0000.to_string());

    // need to initialize oracle, because we don't call `provide_liquidity` in this test
    wyndex::oracle::initialize_oracle(
        &mut deps.storage,
        &mock_env_with_block_time(0),
        Decimal256::one(),
    )
    .unwrap();

    // Normal swap
    let msg = ExecuteMsg::Swap {
        offer_asset: Asset {
            info: AssetInfo::Native("uusd".to_string()),
            amount: offer_amount,
        },
        ask_asset_info: None,
        belief_price: None,
        max_spread: Some(Decimal256::percent(50)),
        to: None,
        referral_address: None,
        referral_commission: None,
    };
    let env = mock_env_with_block_time(1000);
    let info = message_info(
        &addr0000,
        &[Coin {
            denom: "uusd".to_string(),
            amount: offer_amount,
        }],
    );

    let res = execute(deps.as_mut(), env, info, msg).unwrap();
    let msg_transfer = res.messages.get(0).expect("no message");

    // Current price is 1.5, so expected return without spread is 1000
    // 952380952 = 20000000000 - (30000000000 * 20000000000) / (30000000000 + 1500000000)
    let expected_ret_amount = Uint256::new(952_380_952u128);

    // 47619047 = 1500000000 * (20000000000 / 30000000000) - 952380952
    let expected_spread_amount = Uint256::new(47619047u128);

    let expected_commission_amount = expected_ret_amount.multiply_ratio(3u128, 1000u128); // 0.3%
    let expected_protocol_fee_amount = expected_commission_amount.multiply_ratio(166u128, 1000u128); // 0.166

    let expected_return_amount = expected_ret_amount
        .checked_sub(expected_commission_amount)
        .unwrap();

    // Check simulation result
    deps.querier.with_balance(&[(
        &MOCK_CONTRACT_ADDR.to_string(),
        &[Coin {
            denom: "uusd".to_string(),
            amount: collateral_pool_amount, /* user deposit must be pre-applied */
        }],
    )]);

    let err = query_simulation(
        deps.as_ref(),
        Asset {
            info: AssetInfo::Native("cny".to_string()),
            amount: offer_amount,
        },
        false,
        None,
    )
    .unwrap_err();
    assert!(err.to_string().contains("Given offer asset does not belong in the pair"));

    let simulation_res: SimulationResponse = query_simulation(
        deps.as_ref(),
        Asset {
            info: AssetInfo::Native("uusd".to_string()),
            amount: offer_amount,
        },
        false,
        None,
    )
    .unwrap();
    assert_eq!(expected_return_amount, simulation_res.return_amount);
    assert_eq!(expected_commission_amount, simulation_res.commission_amount);
    assert_eq!(expected_spread_amount, simulation_res.spread_amount);

    // Check reverse simulation result
    let err = query_reverse_simulation(
        deps.as_ref(),
        Asset {
            info: AssetInfo::Native("cny".to_string()),
            amount: expected_return_amount,
        },
        false,
        None,
    )
    .unwrap_err();
    assert!(err.to_string().contains("Given ask asset doesn't belong to pairs"));

    let reverse_simulation_res: ReverseSimulationResponse = query_reverse_simulation(
        deps.as_ref(),
        Asset {
            info: AssetInfo::Token(asset0000.to_string()),
            amount: expected_return_amount,
        },
        false,
        None,
    )
    .unwrap();
    assert!(
        (Uint128::try_from(offer_amount).unwrap().u128() as i128
            - Uint128::try_from(reverse_simulation_res.offer_amount)
                .unwrap()
                .u128() as i128)
            .abs()
            < 5i128
    );
    assert!(
        (Uint128::try_from(expected_commission_amount)
            .unwrap()
            .u128() as i128
            - Uint128::try_from(reverse_simulation_res.commission_amount)
                .unwrap()
                .u128() as i128)
            .abs()
            < 5i128
    );
    assert!(
        (Uint128::try_from(expected_spread_amount).unwrap().u128() as i128
            - Uint128::try_from(reverse_simulation_res.spread_amount)
                .unwrap()
                .u128() as i128)
            .abs()
            < 5i128
    );

    assert_eq!(
        res.attributes,
        vec![
            attr("action", "swap"),
            attr("sender", addr0000.to_string()),
            attr("receiver", addr0000.to_string()),
            attr("offer_asset", "uusd"),
            attr("ask_asset", asset0000.to_string()),
            attr("offer_amount", offer_amount.to_string()),
            attr("return_amount", expected_return_amount.to_string()),
            attr("spread_amount", expected_spread_amount.to_string()),
            attr("commission_amount", expected_commission_amount.to_string()),
            attr(
                "protocol_fee_amount",
                expected_protocol_fee_amount.to_string()
            ),
        ]
    );

    assert_eq!(
        &SubMsg {
            msg: WasmMsg::Execute {
                contract_addr: asset0000.to_string(),
                msg: to_json_binary(&Cw20ExecuteMsg::Transfer {
                    recipient: addr0000.to_string(),
                    amount: expected_return_amount,
                })
                .unwrap(),
                funds: vec![],
            }
            .into(),
            id: 0,
            gas_limit: None,
            reply_on: ReplyOn::Never,
            payload: Binary::new(vec![])
        },
        msg_transfer,
    );
}

#[test]
fn try_token_to_native() {
    let total_share = Uint128::new(20000000000u128);
    let asset_pool_amount = Uint128::new(30000000000u128);
    let collateral_pool_amount = Uint256::new(20000000000u128);
    let offer_amount = Uint128::new(1500000000u128);

    let mut deps = mock_dependencies(&[Coin {
        denom: "uusd".to_string(),
        amount: collateral_pool_amount,
    }]);

    let a = TestAccounts::new(&deps.api);
    let liquidity0000 = a.trader;
    let addr0000 = a.whale;
    let asset0000 = a.owner;
    let factory = a.beneficiary;

    deps.querier.with_token_balances(&[
        (
            &liquidity0000.to_string(),
            &[(&MOCK_CONTRACT_ADDR.to_string(), &total_share)],
        ),
        (
            &asset0000.to_string(),
            &[(
                &MOCK_CONTRACT_ADDR.to_string(),
                &(asset_pool_amount + offer_amount),
            )],
        ),
    ]);

    let msg = InstantiateMsg {
        asset_infos: vec![
            AssetInfo::Native("uusd".to_string()),
            AssetInfo::Token(asset0000.to_string()),
        ],
        token_code_id: 10u64,
        factory_addr: factory.to_string(),
        init_params: None,
        staking_config: default_stake_config(),
        trading_starts: 0,
        fee_config: FeeConfig {
            total_fee_bps: 30,
            protocol_fee_bps: 1660,
        },
        circuit_breaker: None,
    };

    let env = mock_env();
    let info = message_info(&addr0000, &[]);
    // We can just call .unwrap() to assert this was a success
    let _res = instantiate(deps.as_mut(), env, info, msg).unwrap();

    // Store liquidity token
    store_liquidity_token(deps.as_mut(), liquidity0000.to_string());

    // need to initialize oracle, because we don't call `provide_liquidity` in this test
    wyndex::oracle::initialize_oracle(
        &mut deps.storage,
        &mock_env_with_block_time(0),
        Decimal256::one(),
    )
    .unwrap();

    // Unauthorized access; can not execute swap directy for token swap
    let msg = ExecuteMsg::Swap {
        offer_asset: Asset {
            info: AssetInfo::Token(asset0000.to_string()),
            amount: offer_amount.into(),
        },
        ask_asset_info: None,
        belief_price: None,
        max_spread: None,
        to: None,
        referral_address: None,
        referral_commission: None,
    };
    let env = mock_env_with_block_time(1000);
    let info = message_info(&addr0000, &[]);
    let res = execute(deps.as_mut(), env, info, msg).unwrap_err();
    assert_eq!(res, ContractError::Unauthorized {});

    // Normal sell
    let msg = ExecuteMsg::Receive(Cw20ReceiveMsg {
        sender: addr0000.to_string(),
        amount: offer_amount.into(),
        msg: to_json_binary(&Cw20HookMsg::Swap {
            ask_asset_info: None,
            belief_price: None,
            max_spread: Some(Decimal256::percent(50)),
            to: None,
            referral_address: None,
            referral_commission: None,
        })
        .unwrap(),
    });
    let env = mock_env_with_block_time(1000);
    let info = message_info(&asset0000, &[]);

    let res = execute(deps.as_mut(), env, info, msg).unwrap();
    let msg_transfer = res.messages.get(0).expect("no message");

    // Current price is 1.5, so expected return without spread is 1000
    // 952380952,3809524 = 20000000000 - (30000000000 * 20000000000) / (30000000000 + 1500000000)
    let expected_ret_amount = Uint256::new(952_380_952u128);

    // 47619047 = 1500000000 * (20000000000 / 30000000000) - 952380952,3809524
    let expected_spread_amount = Uint256::new(47619047u128);

    let expected_commission_amount = expected_ret_amount.multiply_ratio(3u128, 1000u128); // 0.3%
    let expected_protocol_fee_amount = expected_commission_amount.multiply_ratio(166u128, 1000u128);
    let expected_return_amount = expected_ret_amount
        .checked_sub(expected_commission_amount)
        .unwrap();

    // Check simulation res
    // Return asset token balance as normal
    deps.querier.with_token_balances(&[
        (
            &liquidity0000.to_string(),
            &[(&MOCK_CONTRACT_ADDR.to_string(), &total_share)],
        ),
        (
            &asset0000.to_string(),
            &[(&MOCK_CONTRACT_ADDR.to_string(), &(asset_pool_amount))],
        ),
    ]);

    let simulation_res: SimulationResponse = query_simulation(
        deps.as_ref(),
        Asset {
            amount: offer_amount.into(),
            info: AssetInfo::Token(asset0000.to_string()),
        },
        false,
        None,
    )
    .unwrap();
    assert_eq!(expected_return_amount, simulation_res.return_amount);
    assert_eq!(expected_commission_amount, simulation_res.commission_amount);
    assert_eq!(expected_spread_amount, simulation_res.spread_amount);

    // Check reverse simulation result
    let reverse_simulation_res: ReverseSimulationResponse = query_reverse_simulation(
        deps.as_ref(),
        Asset {
            amount: expected_return_amount,
            info: AssetInfo::Native("uusd".to_string()),
        },
        false,
        None,
    )
    .unwrap();
    assert!(
        (offer_amount.u128() as i128
            - Uint128::try_from(reverse_simulation_res.offer_amount)
                .unwrap()
                .u128() as i128)
            .abs()
            < 5i128
    );
    assert!(
        (Uint128::try_from(expected_commission_amount)
            .unwrap()
            .u128() as i128
            - Uint128::try_from(reverse_simulation_res.commission_amount)
                .unwrap()
                .u128() as i128)
            .abs()
            < 5i128
    );
    assert!(
        (Uint128::try_from(expected_spread_amount).unwrap().u128() as i128
            - Uint128::try_from(reverse_simulation_res.spread_amount)
                .unwrap()
                .u128() as i128)
            .abs()
            < 5i128
    );

    assert_eq!(
        res.attributes,
        vec![
            attr("action", "swap"),
            attr("sender", addr0000.to_string()),
            attr("receiver", addr0000.to_string()),
            attr("offer_asset", asset0000.to_string()),
            attr("ask_asset", "uusd"),
            attr("offer_amount", offer_amount.to_string()),
            attr("return_amount", expected_return_amount.to_string()),
            attr("spread_amount", expected_spread_amount.to_string()),
            attr("commission_amount", expected_commission_amount.to_string()),
            attr(
                "protocol_fee_amount",
                expected_protocol_fee_amount.to_string()
            ),
        ]
    );

    assert_eq!(
        &SubMsg {
            msg: CosmosMsg::Bank(BankMsg::Send {
                to_address: addr0000.to_string(),
                amount: vec![Coin {
                    denom: "uusd".to_string(),
                    amount: expected_return_amount
                }],
            }),
            id: 0,
            gas_limit: None,
            reply_on: ReplyOn::Never,
            payload: Binary::new(vec![])
        },
        msg_transfer,
    );

    // Failed due to trying to swap a non token (specifying an address of a non token contract)
    let msg = ExecuteMsg::Receive(Cw20ReceiveMsg {
        sender: addr0000.to_string(),
        amount: offer_amount.into(),
        msg: to_json_binary(&Cw20HookMsg::Swap {
            ask_asset_info: None,
            belief_price: None,
            max_spread: None,
            to: None,
            referral_address: None,
            referral_commission: None,
        })
        .unwrap(),
    });
    let env = mock_env_with_block_time(1000);
    let info = message_info(&Addr::unchecked("liquidtity0000"), &[]);
    let res = execute(deps.as_mut(), env, info, msg).unwrap_err();
    assert_eq!(res, ContractError::Unauthorized {});
}

#[test]
fn test_max_spread() {
    assert_max_spread(
        Some(Decimal256::from_ratio(1200u128, 1u128)),
        Some(Decimal256::percent(1)),
        Uint256::from(1200000000u128),
        Uint256::from(989999u128),
        Uint256::zero(),
    )
    .unwrap_err();

    assert_max_spread(
        Some(Decimal256::from_ratio(1200u128, 1u128)),
        Some(Decimal256::percent(1)),
        Uint256::from(1200000000u128),
        Uint256::from(990000u128),
        Uint256::zero(),
    )
    .unwrap();

    assert_max_spread(
        None,
        Some(Decimal256::percent(1)),
        Uint256::zero(),
        Uint256::from(989999u128),
        Uint256::from(10001u128),
    )
    .unwrap_err();

    assert_max_spread(
        None,
        Some(Decimal256::percent(1)),
        Uint256::zero(),
        Uint256::from(990000u128),
        Uint256::from(10000u128),
    )
    .unwrap();

    assert_max_spread(
        Some(Decimal256::from_ratio(1200u128, 1u128)),
        Some(Decimal256::percent(51)),
        Uint256::from(1200000000u128),
        Uint256::from(989999u128),
        Uint256::zero(),
    )
    .unwrap_err();
}

#[test]
fn test_query_pool() {
    let total_share_amount = Uint256::from(111u128);
    let asset_0_amount = Uint256::from(222u128);
    let asset_1_amount = Uint256::from(333u128);
    let mut deps = mock_dependencies(&[Coin {
        denom: "uusd".to_string(),
        amount: asset_0_amount,
    }]);

    let a = TestAccounts::new(&deps.api);
    let liquidity0000 = a.trader;
    let addr0000 = a.whale;
    let asset0000 = a.owner;
    let factory = a.beneficiary;

    deps.querier.with_token_balances(&[
        (
            &asset0000.to_string(),
            &[(
                &MOCK_CONTRACT_ADDR.to_string(),
                &Uint128::try_from(asset_1_amount).unwrap(),
            )],
        ),
        (
            &liquidity0000.to_string(),
            &[(
                &MOCK_CONTRACT_ADDR.to_string(),
                &Uint128::try_from(total_share_amount).unwrap(),
            )],
        ),
    ]);

    let msg = InstantiateMsg {
        asset_infos: vec![
            AssetInfo::Native("uusd".to_string()),
            AssetInfo::Token(asset0000.to_string()),
        ],
        token_code_id: 10u64,
        factory_addr: factory.to_string(),
        init_params: None,
        staking_config: default_stake_config(),
        trading_starts: 0,
        fee_config: FeeConfig {
            total_fee_bps: 0,
            protocol_fee_bps: 0,
        },
        circuit_breaker: None,
    };

    let env = mock_env();
    let info = message_info(&addr0000, &[]);
    // We can just call .unwrap() to assert this was a success
    let _res = instantiate(deps.as_mut(), env, info, msg).unwrap();

    // Store liquidity token
    store_liquidity_token(deps.as_mut(), liquidity0000.to_string());

    let res: PoolResponse = query_pool(deps.as_ref()).unwrap();

    assert_eq!(
        res.assets,
        [
            AssetValidated {
                info: AssetInfoValidated::Native("uusd".to_string()),
                amount: asset_0_amount
            },
            AssetValidated {
                info: AssetInfoValidated::Token(asset0000),
                amount: asset_1_amount
            }
        ]
    );
    assert_eq!(res.total_share, total_share_amount);
}

#[test]
fn test_query_share() {
    let total_share_amount = Uint128::from(500u128);
    let asset_0_amount = Uint128::from(250u128);
    let asset_1_amount = Uint128::from(1000u128);
    let mut deps = mock_dependencies(&[Coin {
        denom: "uusd".to_string(),
        amount: asset_0_amount.into(),
    }]);

    let a = TestAccounts::new(&deps.api);
    let liquidity0000 = a.trader;
    let addr0000 = a.whale;
    let asset0000 = a.owner;
    let factory = a.beneficiary;

    deps.querier.with_token_balances(&[
        (
            &asset0000.to_string(),
            &[(&MOCK_CONTRACT_ADDR.to_string(), &asset_1_amount)],
        ),
        (
            &liquidity0000.to_string(),
            &[(&MOCK_CONTRACT_ADDR.to_string(), &total_share_amount)],
        ),
    ]);

    let msg = InstantiateMsg {
        asset_infos: vec![
            AssetInfo::Native("uusd".to_string()),
            AssetInfo::Token(asset0000.to_string()),
        ],
        token_code_id: 10u64,
        factory_addr: factory.to_string(),
        init_params: None,
        staking_config: default_stake_config(),
        trading_starts: 0,
        fee_config: FeeConfig {
            total_fee_bps: 0,
            protocol_fee_bps: 0,
        },
        circuit_breaker: None,
    };

    let env = mock_env();
    let info = message_info(&addr0000, &[]);
    // We can just call .unwrap() to assert this was a success
    let _res = instantiate(deps.as_mut(), env, info, msg).unwrap();

    // Store liquidity token
    store_liquidity_token(deps.as_mut(), liquidity0000.to_string());

    let res = query_share(deps.as_ref(), Uint256::new(250)).unwrap();

    assert_eq!(res[0].amount, Uint256::new(125));
    assert_eq!(res[1].amount, Uint256::new(500));
}

#[test]
fn test_accumulate_prices() {
    struct Case {
        block_time: u64,
        block_time_last: u64,
        last0: u128,
        last1: u128,
        x_amount: u128,
        y_amount: u128,
    }

    struct Result {
        block_time_last: u64,
        price_x: u128,
        price_y: u128,
        is_some: bool,
    }

    let price_precision = 10u128.pow(TWAP_PRECISION.into());

    let test_cases: Vec<(Case, Result)> = vec![
        (
            Case {
                block_time: 1000,
                block_time_last: 0,
                last0: 0,
                last1: 0,
                x_amount: 250,
                y_amount: 500,
            },
            Result {
                block_time_last: 1000,
                price_x: 2000, // 500/250*1000
                price_y: 500,  // 250/500*1000
                is_some: true,
            },
        ),
        // Same block height, no changes
        (
            Case {
                block_time: 1000,
                block_time_last: 1000,
                last0: price_precision,
                last1: 2 * price_precision,
                x_amount: 250,
                y_amount: 500,
            },
            Result {
                block_time_last: 1000,
                price_x: 1,
                price_y: 2,
                is_some: false,
            },
        ),
        (
            Case {
                block_time: 1500,
                block_time_last: 1000,
                last0: 500 * price_precision,
                last1: 2000 * price_precision,
                x_amount: 250,
                y_amount: 500,
            },
            Result {
                block_time_last: 1500,
                price_x: 1500, // 500 + (500/250*500)
                price_y: 2250, // 2000 + (250/500*500)
                is_some: true,
            },
        ),
    ];

    for test_case in test_cases {
        let (case, result) = test_case;
        let a = TestAccounts::new(&App::default().api());
        let addr0000 = a.whale;
        let asset0000 = a.owner;
        let factory = a.beneficiary;

        let env = mock_env_with_block_time(case.block_time);
        let config = accumulate_prices(
            &env,
            &Config {
                pair_info: PairInfo {
                    asset_infos: vec![
                        AssetInfoValidated::Native("uusd".to_string()),
                        AssetInfoValidated::Token(asset0000),
                    ],
                    contract_addr: Addr::unchecked("pair"),
                    staking_addr: Addr::unchecked("stake"),
                    liquidity_token: Addr::unchecked("lp_token"),
                    pair_type: PairType::Xyk {}, // Implemented in mock querier
                    fee_config: FeeConfig {
                        total_fee_bps: 0,
                        protocol_fee_bps: 0,
                    },
                },
                factory_addr: factory,
                block_time_last: case.block_time_last,
                price0_cumulative_last: Uint256::new(case.last0),
                price1_cumulative_last: Uint256::new(case.last1),
                trading_starts: 0,
            },
            Uint256::new(case.x_amount),
            Uint256::new(case.y_amount),
        )
        .unwrap();

        assert_eq!(result.is_some, config.is_some());

        if let Some(config) = config {
            assert_eq!(config.2, result.block_time_last);
            assert_eq!(
                config.0 / Uint256::from(price_precision),
                Uint256::new(result.price_x)
            );
            assert_eq!(
                config.1 / Uint256::from(price_precision),
                Uint256::new(result.price_y)
            );
        }
    }
}

fn mock_env_with_block_time(time: u64) -> Env {
    let mut env = mock_env();
    env.block = BlockInfo {
        height: 1,
        time: Timestamp::from_seconds(time),
        chain_id: "columbus".to_string(),
    };
    env
}

#[test]
fn compute_swap_rounding() {
    let offer_pool = Uint256::from(5_000_000_000_000_u128);
    let ask_pool = Uint256::from(1_000_000_000_u128);
    let return_amount = Uint256::from(0_u128);
    let spread_amount = Uint256::from(0_u128);
    let commission_amount = Uint256::from(0_u128);
    let offer_amount = Uint256::from(1_u128);

    assert_eq!(
        compute_swap(offer_pool, ask_pool, offer_amount, Decimal256::zero()).unwrap(),
        (return_amount, spread_amount, commission_amount)
    );
}

proptest! {
    #[test]
    fn compute_swap_overflow_test(
        offer_pool in 1_000_000..9_000_000_000_000_000_000u128,
        ask_pool in 1_000_000..9_000_000_000_000_000_000u128,
        offer_amount in 1..100_000_000_000u128,
    ) {

        let offer_pool = Uint256::from(offer_pool);
        let ask_pool = Uint256::from(ask_pool);
        let offer_amount = Uint256::from(offer_amount);
        let commission_amount = Decimal256::zero();

        // Make sure there are no overflows
        compute_swap(
            offer_pool,
            ask_pool,
            offer_amount,
            commission_amount,
        ).unwrap();
    }
}

#[test]
fn ensure_useful_error_messages_are_given_on_swaps() {
    const OFFER: Uint256 = Uint256::new(1_000_000_000_000);
    const ASK: Uint256 = Uint256::new(1_000_000_000_000);
    const AMOUNT: Uint256 = Uint256::new(1_000_000);
    const ZERO: Uint256 = Uint256::zero();
    const DZERO: Decimal256 = Decimal256::zero();

    // Computing ask
    assert_eq!(
        compute_swap(ZERO, ZERO, ZERO, DZERO)
            .unwrap_err()
            .to_string(),
        StdError::msg("One of the pools is empty").to_string()
    );
    assert_eq!(
        compute_swap(ZERO, ZERO, AMOUNT, DZERO)
            .unwrap_err()
            .to_string(),
        StdError::msg("One of the pools is empty").to_string()
    );
    assert_eq!(
        compute_swap(ZERO, ASK, ZERO, DZERO)
            .unwrap_err()
            .to_string(),
        StdError::msg("One of the pools is empty").to_string()
    );
    assert_eq!(
        compute_swap(ZERO, ASK, AMOUNT, DZERO)
            .unwrap_err()
            .to_string(),
        StdError::msg("One of the pools is empty").to_string()
    );
    assert_eq!(
        compute_swap(OFFER, ZERO, ZERO, DZERO)
            .unwrap_err()
            .to_string(),
        StdError::msg("One of the pools is empty").to_string()
    );
    assert_eq!(
        compute_swap(OFFER, ZERO, AMOUNT, DZERO)
            .unwrap_err()
            .to_string(),
        StdError::msg("One of the pools is empty").to_string()
    );
    assert_eq!(
        compute_swap(OFFER, ASK, ZERO, DZERO)
            .unwrap_err()
            .to_string(),
        StdError::msg("Swap amount must not be zero").to_string()
    );
    compute_swap(OFFER, ASK, AMOUNT, DZERO).unwrap();

    // Computing offer
    assert_eq!(
        compute_offer_amount(ZERO, ZERO, ZERO, DZERO)
            .unwrap_err()
            .to_string(),
        StdError::msg("One of the pools is empty").to_string()
    );
    assert_eq!(
        compute_offer_amount(ZERO, ZERO, AMOUNT, DZERO)
            .unwrap_err()
            .to_string(),
        StdError::msg("One of the pools is empty").to_string()
    );
    assert_eq!(
        compute_offer_amount(ZERO, ASK, ZERO, DZERO)
            .unwrap_err()
            .to_string(),
        StdError::msg("One of the pools is empty").to_string()
    );
    assert_eq!(
        compute_offer_amount(ZERO, ASK, AMOUNT, DZERO)
            .unwrap_err()
            .to_string(),
        StdError::msg("One of the pools is empty").to_string()
    );
    assert_eq!(
        compute_offer_amount(OFFER, ZERO, ZERO, DZERO)
            .unwrap_err()
            .to_string(),
        StdError::msg("One of the pools is empty").to_string()
    );
    assert_eq!(
        compute_offer_amount(OFFER, ZERO, AMOUNT, DZERO)
            .unwrap_err()
            .to_string(),
        StdError::msg("One of the pools is empty").to_string()
    );
    assert_eq!(
        compute_offer_amount(OFFER, ASK, ZERO, DZERO)
            .unwrap_err()
            .to_string(),
        StdError::msg("Swap amount must not be zero").to_string()
    );
    compute_offer_amount(OFFER, ASK, AMOUNT, DZERO).unwrap();
}
