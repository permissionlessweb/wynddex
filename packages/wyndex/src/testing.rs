#[cfg(test)]
mod tests {
    use crate::asset::{format_lp_token_name, AssetInfo, AssetInfoValidated, AssetValidated};
    use crate::fee_config::FeeConfig;
    use crate::mock_querier::mock_dependencies;
    use crate::pair::PairInfo;
    use crate::querier::{query_balance, query_pair_info, query_supply, query_token_balance};

    use crate::factory::PairType;
    use cosmwasm_std::testing::{MockApi, MOCK_CONTRACT_ADDR};
    use cosmwasm_std::{
        to_json_binary, Addr, BankMsg, Coin, CosmosMsg, Decimal, Decimal256, Uint128, Uint256,
        WasmMsg,
    };
    use cw20::Cw20ExecuteMsg;
    use wyndex_test_helpers::TestAccounts;

    #[test]
    fn token_balance_querier() {
        let mut deps = mock_dependencies(&[]);

        let a = TestAccounts::new(&deps.api);
        let sender = a.user;
        let liquidity0000 = a.trader;
        let addr0000 = a.whale;
        let asset0000 = a.owner;
        let factory = a.beneficiary;

        deps.querier.with_token_balances(&[(
            &liquidity0000.to_string(),
            &[(&MOCK_CONTRACT_ADDR.to_string(), &Uint128::new(123u128))],
        )]);

        deps.querier.with_cw20_query_handler();
        assert_eq!(
            Uint256::new(123u128),
            query_token_balance(&deps.as_ref().querier, liquidity0000, MOCK_CONTRACT_ADDR,)
                .unwrap()
        );
        deps.querier.with_default_query_handler()
    }

    #[test]
    fn balance_querier() {
        let deps = mock_dependencies(&[Coin {
            denom: "uusd".to_string(),
            amount: Uint256::new(200u128),
        }]);

        assert_eq!(
            query_balance(
                &deps.as_ref().querier,
                MOCK_CONTRACT_ADDR,
                "uusd".to_string()
            )
            .unwrap(),
            Uint256::new(200u128)
        );
    }

    #[test]
    fn all_balances_querier() {
        let _deps = mock_dependencies(&[
            Coin {
                denom: "uusd".to_string(),
                amount: Uint256::new(200u128),
            },
            Coin {
                denom: "ukrw".to_string(),
                amount: Uint256::new(300u128),
            },
        ]);
    }

    #[test]
    fn supply_querier() {
        let mut deps = mock_dependencies(&[]);

        let a = TestAccounts::new(&deps.api);
        let sender = a.user;
        let liquidity0000 = a.trader;

        deps.querier.with_token_balances(&[(
            &liquidity0000.to_string(),
            &[
                (&MOCK_CONTRACT_ADDR.to_string(), &Uint128::new(123u128)),
                (
                    &MockApi::default().addr_make("addr00000").to_string(),
                    &Uint128::new(123u128),
                ),
                (
                    &MockApi::default().addr_make("addr00001").to_string(),
                    &Uint128::new(123u128),
                ),
                (
                    &MockApi::default().addr_make("addr00002").to_string(),
                    &Uint128::new(123u128),
                ),
            ],
        )]);

        deps.querier.with_cw20_query_handler();

        assert_eq!(
            query_supply(&deps.as_ref().querier, liquidity0000).unwrap(),
            Uint256::new(492u128)
        )
    }

    #[test]
    fn test_asset_info() {
        let a = TestAccounts::new(&MockApi::default());
        let asset0000 = a.owner;
        let asset0001 = a.fee_receiver;

        let token_info: AssetInfoValidated = AssetInfoValidated::Token(asset0000.clone());
        let native_token_info: AssetInfoValidated = AssetInfoValidated::Native("uusd".to_string());

        assert!(!token_info.equal(&native_token_info));

        assert!(!token_info.equal(&AssetInfoValidated::Token(asset0001)));

        assert!(token_info.equal(&AssetInfoValidated::Token(asset0000.clone())));

        assert!(native_token_info.is_native_token());
        assert!(!token_info.is_native_token());

        let mut deps = mock_dependencies(&[Coin {
            denom: "uusd".to_string(),
            amount: Uint256::new(123),
        }]);
        deps.querier.with_token_balances(&[(
            &asset0000.to_string(),
            &[
                (&MOCK_CONTRACT_ADDR.to_string(), &Uint128::new(123u128)),
                (
                    &MockApi::default().addr_make("addr00000").to_string(),
                    &Uint128::new(123u128),
                ),
                (
                    &MockApi::default().addr_make("addr00001").to_string(),
                    &Uint128::new(123u128),
                ),
                (
                    &MockApi::default().addr_make("addr00002").to_string(),
                    &Uint128::new(123u128),
                ),
            ],
        )]);

        assert_eq!(
            native_token_info
                .query_balance(&deps.as_ref().querier, MOCK_CONTRACT_ADDR,)
                .unwrap(),
            Uint256::new(123u128)
        );
        deps.querier.with_cw20_query_handler();
        assert_eq!(
            token_info
                .query_balance(&deps.as_ref().querier, MOCK_CONTRACT_ADDR,)
                .unwrap(),
            Uint256::new(123u128)
        );
    }

    #[test]
    fn test_asset() {
        let mut deps = mock_dependencies(&[Coin {
            denom: "uusd".to_string(),
            amount: Uint256::new(123),
        }]);
        let a = TestAccounts::new(&MockApi::default());

        let addr0000 = a.whale;
        let asset0000 = a.owner;

        deps.querier.with_token_balances(&[(
            &asset0000.to_string(),
            &[
                (&MOCK_CONTRACT_ADDR.to_string(), &Uint128::new(123u128)),
                (
                    &MockApi::default().addr_make("addr00000").to_string(),
                    &Uint128::new(123u128),
                ),
                (
                    &MockApi::default().addr_make("addr00001").to_string(),
                    &Uint128::new(123u128),
                ),
                (
                    &MockApi::default().addr_make("addr00002").to_string(),
                    &Uint128::new(123u128),
                ),
            ],
        )]);

        let token_asset = AssetValidated {
            amount: Uint256::new(123123u128),
            info: AssetInfoValidated::Token(asset0000.clone()),
        };

        let native_token_asset = AssetValidated {
            amount: Uint256::new(123123u128),
            info: AssetInfoValidated::Native("uusd".to_string()),
        };

        assert_eq!(
            token_asset.into_msg(&addr0000).unwrap(),
            CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: asset0000.to_string(),
                msg: to_json_binary(&Cw20ExecuteMsg::Transfer {
                    recipient: addr0000.to_string(),
                    amount: Uint256::new(123123u128),
                })
                .unwrap(),
                funds: vec![],
            })
        );

        assert_eq!(
            native_token_asset.into_msg(&addr0000).unwrap(),
            CosmosMsg::Bank(BankMsg::Send {
                to_address: addr0000.to_string(),
                amount: vec![Coin {
                    denom: "uusd".to_string(),
                    amount: Uint256::new(123123u128),
                }]
            })
        );
    }

    #[test]
    fn query_wyndex_pair_contract() {
        let mut deps = mock_dependencies(&[]);
        let mut a = TestAccounts::new(&MockApi::default());
        let pair0000 = a.addr("pair0000");
        let stake0000 = a.addr("stake0000");
        let liquidity0000 = a.trader;
        let asset0000 = a.owner;
        let pair_key = asset0000.to_string() + "uusd";

        deps.querier.with_wyndex_pairs(&[(
            &pair_key,
            &PairInfo {
                asset_infos: vec![
                    AssetInfoValidated::Token(asset0000.clone()),
                    AssetInfoValidated::Native("uusd".to_string()),
                ],
                contract_addr: pair0000.clone(),
                staking_addr: stake0000,
                liquidity_token: liquidity0000.clone(),
                pair_type: PairType::Xyk {},
                fee_config: FeeConfig {
                    protocol_fee_bps: 0,
                    total_fee_bps: 0,
                },
            },
        )]);

        let pair_info: PairInfo = query_pair_info(
            &deps.as_ref().querier,
            MOCK_CONTRACT_ADDR,
            &[
                AssetInfo::Token(asset0000.to_string()),
                AssetInfo::Native("uusd".to_string()),
            ],
        )
        .unwrap();

        assert_eq!(pair_info.contract_addr.to_string(), pair0000.to_string(),);
        assert_eq!(
            pair_info.liquidity_token.to_string(),
            liquidity0000.to_string(),
        );
    }

    #[test]
    fn test_format_lp_token_name() {
        let mut deps = mock_dependencies(&[]);
        let mut a = TestAccounts::new(&MockApi::default());
        let pair0000 = a.addr("pair0000");
        let stake0000 = a.addr("stake0000");
        let liquidity0000 = a.trader;
        let asset0000 = a.owner;
        let pair_key = asset0000.to_string() + "uusd";

        deps.querier.with_wyndex_pairs(&[(
            &pair_key,
            &PairInfo {
                asset_infos: vec![
                    AssetInfoValidated::Token(asset0000.clone()),
                    AssetInfoValidated::Native("uusd".to_string()),
                ],
                contract_addr: pair0000,
                staking_addr: stake0000,
                liquidity_token: liquidity0000,
                pair_type: PairType::Xyk {},
                fee_config: FeeConfig {
                    protocol_fee_bps: 0,
                    total_fee_bps: 0,
                },
            },
        )]);

        let pair_info: PairInfo = query_pair_info(
            &deps.as_ref().querier,
            MOCK_CONTRACT_ADDR,
            &[
                AssetInfo::Token(asset0000.to_string()),
                AssetInfo::Native("uusd".to_string()),
            ],
        )
        .unwrap();

        deps.querier.with_token_balances(&[(
            &asset0000.to_string(),
            &[(&MOCK_CONTRACT_ADDR.to_string(), &Uint128::new(123u128))],
        )]);

        deps.querier.with_cw20_query_handler();

        let lp_name = format_lp_token_name(&pair_info.asset_infos, &deps.as_ref().querier).unwrap();
        assert_eq!(lp_name, "MAPP-UUSD-LP")
    }

    #[test]
    fn test_decimal_checked_ops() {
        for i in 0u32..100u32 {
            let dec = Decimal::from_ratio(i, 1u32);
            assert_eq!(dec + dec, dec.checked_add(dec).unwrap());
        }
        assert!(
            Decimal256::from_ratio(Uint256::MAX, Uint256::from(10u128.pow(18u32)))
                .checked_add(Decimal256::one())
                .is_err()
        );

        assert!(
            Decimal256::from_ratio(Uint256::MAX, Uint256::from(10u128.pow(18u32)))
                .checked_mul(Decimal256::new(Uint256::from(10u128.pow(18u32) + 1u128)))
                .is_err()
        );
    }
}
