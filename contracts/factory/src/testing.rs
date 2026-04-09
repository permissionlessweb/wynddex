use cosmwasm_std::{
    attr, from_json, to_json_binary, Addr, Binary, Decimal256, ReplyOn, SubMsg, Uint256, WasmMsg,
};
use cw_utils::MsgInstantiateContractResponse;
use wyndex::fee_config::FeeConfig;
use wyndex_test_helpers::TestAccounts;

use crate::mock_querier::mock_dependencies;
use crate::state::CONFIG;
use crate::{
    contract::{execute, instantiate, query},
    error::ContractError,
};
use wyndex::asset::AssetInfo;
use wyndex::factory::{
    ConfigResponse, DefaultStakeConfig, ExecuteMsg, InstantiateMsg, PairConfig, PairType,
    PairsResponse, PartialStakeConfig, QueryMsg,
};
use wyndex::pair::PairInfo;

use crate::contract::reply;
use cosmwasm_std::testing::{message_info, mock_env, MockApi, MOCK_CONTRACT_ADDR};
use wyndex::pair::InstantiateMsg as PairInstantiateMsg;

fn default_stake_config() -> DefaultStakeConfig {
    DefaultStakeConfig {
        staking_code_id: 1234u64,
        tokens_per_power: Uint256::from(1000u128),
        min_bond: Uint256::from(1000u128),
        unbonding_periods: vec![1],
        max_distributions: 6,
        converter: None,
    }
}

fn default_accounts() -> [Addr; 1] {
    let d = mock_dependencies(&[]);
    [d.api.addr_make("owner0000")]
}

#[test]
fn pair_type_to_string() {
    assert_eq!(PairType::Xyk {}.to_string(), "xyk");
    assert_eq!(PairType::Stable {}.to_string(), "stable");
    assert_eq!(PairType::Lsd {}.to_string(), "lsd");
}

#[test]
fn proper_initialization() {
    // Validate total and protocol fee bps
    let mut deps = mock_dependencies(&[]);
    let owner = default_accounts();
    let owner = &owner[0];

    let msg = InstantiateMsg {
        pair_configs: vec![
            PairConfig {
                code_id: 123u64,
                pair_type: PairType::Xyk {},
                fee_config: FeeConfig {
                    total_fee_bps: 100,
                    protocol_fee_bps: 10,
                },
                is_disabled: false,
            },
            PairConfig {
                code_id: 325u64,
                pair_type: PairType::Xyk {},
                fee_config: FeeConfig {
                    total_fee_bps: 100,
                    protocol_fee_bps: 10,
                },
                is_disabled: false,
            },
        ],
        token_code_id: 123u64,
        fee_address: None,
        owner: owner.to_string(),
        max_referral_commission: Decimal256::one(),
        default_stake_config: default_stake_config(),
        trading_starts: None,
    };

    let env = mock_env();
    let info = message_info(&owner, &[]);

    let res = instantiate(deps.as_mut(), env, info, msg).unwrap_err();
    assert_eq!(res, ContractError::PairConfigDuplicate {});

    let msg = InstantiateMsg {
        pair_configs: vec![PairConfig {
            code_id: 123u64,
            pair_type: PairType::Xyk {},
            fee_config: FeeConfig {
                total_fee_bps: 10_001,
                protocol_fee_bps: 10,
            },
            is_disabled: false,
        }],
        token_code_id: 123u64,
        fee_address: None,
        owner: owner.to_string(),
        max_referral_commission: Decimal256::one(),
        default_stake_config: default_stake_config(),
        trading_starts: None,
    };

    let env = mock_env();
    let info = message_info(&owner, &[]);

    let res = instantiate(deps.as_mut(), env, info, msg).unwrap_err();
    assert_eq!(res, ContractError::PairConfigInvalidFeeBps {});

    let mut deps = mock_dependencies(&[]);

    let msg = InstantiateMsg {
        pair_configs: vec![
            PairConfig {
                code_id: 325u64,
                pair_type: PairType::Lsd {},
                fee_config: FeeConfig {
                    total_fee_bps: 100,
                    protocol_fee_bps: 10,
                },
                is_disabled: false,
            },
            PairConfig {
                code_id: 123u64,
                pair_type: PairType::Xyk {},
                fee_config: FeeConfig {
                    total_fee_bps: 100,
                    protocol_fee_bps: 10,
                },
                is_disabled: false,
            },
        ],
        token_code_id: 123u64,
        fee_address: None,
        owner: owner.to_string(),
        max_referral_commission: Decimal256::one(),
        default_stake_config: default_stake_config(),
        trading_starts: None,
    };

    let env = mock_env();
    let info = message_info(&owner, &[]);

    instantiate(deps.as_mut(), env.clone(), info, msg.clone()).unwrap();

    let query_res = query(deps.as_ref(), env, QueryMsg::Config {}).unwrap();
    let config_res: ConfigResponse = from_json(&query_res).unwrap();
    assert_eq!(123u64, config_res.token_code_id);
    assert_eq!(msg.pair_configs, config_res.pair_configs);
    assert_eq!(owner, config_res.owner);
}

#[test]
fn trading_starts_validation() {
    let mut deps = mock_dependencies(&[]);
    let env = mock_env();
    let addr0000 = MockApi::default().addr_make("addr0000");
    let info = message_info(&addr0000, &[]);

    let owner = MockApi::default().addr_make("owner").to_string();

    let mut msg = InstantiateMsg {
        pair_configs: vec![],
        token_code_id: 123u64,
        fee_address: None,
        owner: owner.to_string(),
        max_referral_commission: Decimal256::one(),
        default_stake_config: default_stake_config(),
        trading_starts: None,
    };

    // in the past
    msg.trading_starts = Some(env.block.time.seconds() - 1);
    let res = instantiate(deps.as_mut(), env.clone(), info.clone(), msg.clone()).unwrap_err();
    assert_eq!(res, ContractError::InvalidTradingStart {});

    const SECONDS_PER_DAY: u64 = 60 * 60 * 24;
    // too late
    msg.trading_starts = Some(env.block.time.seconds() + 60 * SECONDS_PER_DAY + 1);
    let res = instantiate(deps.as_mut(), env.clone(), info.clone(), msg.clone()).unwrap_err();
    assert_eq!(res, ContractError::InvalidTradingStart {});

    // just before too late
    msg.trading_starts = Some(env.block.time.seconds() + 60 * SECONDS_PER_DAY);
    instantiate(deps.as_mut(), env.clone(), info.clone(), msg.clone()).unwrap();

    // right now
    msg.trading_starts = Some(env.block.time.seconds());
    instantiate(deps.as_mut(), env, info, msg).unwrap();
}

#[test]
fn update_config() {
    let mut deps = mock_dependencies(&[]);
    let a = TestAccounts::new(&deps.api);
    let addr0000 = a.whale;
    let new_fee_addr = a.beneficiary;
    let owner = a.owner;

    let pair_configs = vec![PairConfig {
        code_id: 123u64,
        pair_type: PairType::Xyk {},
        fee_config: FeeConfig {
            total_fee_bps: 3,
            protocol_fee_bps: 166,
        },
        is_disabled: false,
    }];

    let msg = InstantiateMsg {
        pair_configs,
        token_code_id: 123u64,
        fee_address: None,
        owner: owner.to_string(),
        max_referral_commission: Decimal256::one(),
        default_stake_config: default_stake_config(),
        trading_starts: None,
    };

    let env = mock_env();
    let info = message_info(&owner, &[]);

    // We can just call .unwrap() to assert this was a success
    let _res = instantiate(deps.as_mut(), env, info, msg).unwrap();

    // Update config
    let env = mock_env();
    let info = message_info(&owner, &[]);
    let msg = ExecuteMsg::UpdateConfig {
        token_code_id: Some(200u64),
        fee_address: Some(new_fee_addr.to_string()),
        only_owner_can_create_pairs: Some(true),
        default_stake_config: None,
    };

    let res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();
    assert_eq!(0, res.messages.len());

    // It worked, let's query the state
    let query_res = query(deps.as_ref(), env, QueryMsg::Config {}).unwrap();
    let config_res: ConfigResponse = from_json(&query_res).unwrap();
    assert_eq!(200u64, config_res.token_code_id);
    assert_eq!(owner.as_str(), config_res.owner.as_str());
    assert_eq!(
        new_fee_addr.as_str(),
        config_res.fee_address.unwrap().as_str()
    );

    // Unauthorized err
    let env = mock_env();
    let info = message_info(&addr0000, &[]);
    let msg = ExecuteMsg::UpdateConfig {
        token_code_id: None,
        fee_address: None,
        only_owner_can_create_pairs: None,
        default_stake_config: None,
    };

    let res = execute(deps.as_mut(), env, info, msg).unwrap_err();
    assert_eq!(res, ContractError::Unauthorized {});
}

#[test]
fn update_owner() {
    let mut deps = mock_dependencies(&[]);
    let owner = MockApi::default().addr_make("owner0000");

    let msg = InstantiateMsg {
        pair_configs: vec![],
        token_code_id: 123u64,
        fee_address: None,
        owner: owner.to_string(),
        max_referral_commission: Decimal256::one(),
        default_stake_config: default_stake_config(),
        trading_starts: None,
    };

    let env = mock_env();
    let info = message_info(&owner, &[]);

    // We can just call .unwrap() to assert this was a success
    instantiate(deps.as_mut(), env, info, msg).unwrap();

    let new_owner = MockApi::default().addr_make("new_owner");

    // New owner
    let env = mock_env();
    let msg = ExecuteMsg::ProposeNewOwner {
        owner: new_owner.to_string(),
        expires_in: 100, // seconds
    };

    let info = message_info(&new_owner, &[]);

    // Unauthorized check
    let err = execute(deps.as_mut(), env.clone(), info, msg.clone()).unwrap_err();
    assert!(err.to_string().contains("Unauthorized"));

    // Claim before proposal
    let info = message_info(&new_owner, &[]);
    execute(
        deps.as_mut(),
        env.clone(),
        info,
        ExecuteMsg::ClaimOwnership {},
    )
    .unwrap_err();

    // Propose new owner
    let info = message_info(&owner, &[]);
    let res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();
    assert_eq!(0, res.messages.len());

    // Unauthorized ownership claim
    let info = message_info(&Addr::unchecked("invalid_addr"), &[]);
    let err = execute(
        deps.as_mut(),
        env.clone(),
        info,
        ExecuteMsg::ClaimOwnership {},
    )
    .unwrap_err();
    assert!(err.to_string().contains("Unauthorized"));

    // Claim ownership
    let info = message_info(&new_owner, &[]);
    let res = execute(
        deps.as_mut(),
        env.clone(),
        info,
        ExecuteMsg::ClaimOwnership {},
    )
    .unwrap();
    assert_eq!(0, res.messages.len());

    // Let's query the state
    let config: ConfigResponse =
        from_json(&query(deps.as_ref(), env, QueryMsg::Config {}).unwrap()).unwrap();
    assert_eq!(new_owner.as_str(), config.owner.as_str());
}

#[test]
fn update_pair_config() {
    let mut deps = mock_dependencies(&[]);
    let a = TestAccounts::new(&deps.api);
    let addr0000 = a.whale;

    let owner = a.owner;
    let pair_configs = vec![PairConfig {
        code_id: 123u64,
        pair_type: PairType::Xyk {},
        fee_config: FeeConfig {
            total_fee_bps: 100,
            protocol_fee_bps: 10,
        },
        is_disabled: false,
    }];

    let msg = InstantiateMsg {
        pair_configs: pair_configs.clone(),
        token_code_id: 123u64,
        fee_address: None,
        owner: owner.to_string(),
        max_referral_commission: Decimal256::one(),
        default_stake_config: default_stake_config(),
        trading_starts: None,
    };

    let env = mock_env();
    let info = message_info(&addr0000, &[]);

    // We can just call .unwrap() to assert this was a success
    instantiate(deps.as_mut(), env.clone(), info, msg).unwrap();

    // It worked, let's query the state
    let query_res = query(deps.as_ref(), env, QueryMsg::Config {}).unwrap();
    let config_res: ConfigResponse = from_json(&query_res).unwrap();
    assert_eq!(pair_configs, config_res.pair_configs);

    // Update config
    let pair_config = PairConfig {
        code_id: 800,
        pair_type: PairType::Xyk {},
        fee_config: FeeConfig {
            total_fee_bps: 1,
            protocol_fee_bps: 2,
        },
        is_disabled: false,
    };

    // Unauthorized err
    let env = mock_env();
    let info = message_info(&Addr::unchecked("wrong-addr0000"), &[]);
    let msg = ExecuteMsg::UpdatePairConfig {
        config: pair_config.clone(),
    };

    let res = execute(deps.as_mut(), env, info, msg).unwrap_err();
    assert_eq!(res, ContractError::Unauthorized {});

    // Check validation of total and protocol fee bps
    let env = mock_env();
    let info = message_info(&owner, &[]);
    let msg = ExecuteMsg::UpdatePairConfig {
        config: PairConfig {
            code_id: 123u64,
            pair_type: PairType::Xyk {},
            fee_config: FeeConfig {
                total_fee_bps: 3,
                protocol_fee_bps: 10_001,
            },
            is_disabled: false,
        },
    };

    let res = execute(deps.as_mut(), env.clone(), info, msg).unwrap_err();
    assert_eq!(res, ContractError::PairConfigInvalidFeeBps {});

    let info = message_info(&owner, &[]);
    let msg = ExecuteMsg::UpdatePairConfig {
        config: pair_config.clone(),
    };

    let res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();
    assert_eq!(0, res.messages.len());

    // It worked, let's query the state
    let query_res = query(deps.as_ref(), env.clone(), QueryMsg::Config {}).unwrap();
    let config_res: ConfigResponse = from_json(&query_res).unwrap();
    assert_eq!(vec![pair_config.clone()], config_res.pair_configs);

    // Add second config
    let pair_config_custom = PairConfig {
        code_id: 100,
        pair_type: PairType::Custom("test".to_string()),
        fee_config: FeeConfig {
            total_fee_bps: 10,
            protocol_fee_bps: 20,
        },
        is_disabled: false,
    };

    let info = message_info(&owner, &[]);
    let msg = ExecuteMsg::UpdatePairConfig {
        config: pair_config_custom.clone(),
    };

    execute(deps.as_mut(), env.clone(), info, msg).unwrap();

    // It worked, let's query the state
    let query_res = query(deps.as_ref(), env, QueryMsg::Config {}).unwrap();
    let config_res: ConfigResponse = from_json(&query_res).unwrap();
    assert_eq!(
        vec![pair_config_custom, pair_config],
        config_res.pair_configs
    );
}

#[test]
fn create_pair() {
    let mut deps = mock_dependencies(&[]);

    let pair_config = PairConfig {
        code_id: 321u64,
        pair_type: PairType::Xyk {},
        fee_config: FeeConfig {
            total_fee_bps: 100,
            protocol_fee_bps: 10,
        },
        is_disabled: false,
    };

    let owner0000 = MockApi::default().addr_make("owner0000");
    let msg = InstantiateMsg {
        pair_configs: vec![pair_config.clone()],
        token_code_id: 123u64,
        fee_address: None,
        owner: owner0000.to_string(),
        max_referral_commission: Decimal256::one(),
        default_stake_config: default_stake_config(),
        trading_starts: None,
    };

    let env = mock_env();
    let addr0000 = MockApi::default().addr_make("addr0000");
    let info = message_info(&addr0000, &[]);

    // We can just call .unwrap() to assert this was a success
    let _res = instantiate(deps.as_mut(), env, info, msg.clone()).unwrap();

    let asset0000 = MockApi::default().addr_make("asset0000").to_string();
    let asset0001 = MockApi::default().addr_make("asset0001").to_string();
    let asset_infos = vec![
        AssetInfo::Token(asset0000.clone()),
        AssetInfo::Token(asset0001.clone()),
    ];

    let config = CONFIG.load(&deps.storage);
    let env = mock_env();
    let info = message_info(&owner0000, &[]);

    // Check pair creation using a non-whitelisted pair ID
    let res = execute(
        deps.as_mut(),
        env.clone(),
        info.clone(),
        ExecuteMsg::CreatePair {
            pair_type: PairType::Lsd {},
            asset_infos: asset_infos.clone(),
            init_params: None,
            total_fee_bps: None,
            staking_config: PartialStakeConfig::default(),
        },
    )
    .unwrap_err();
    assert_eq!(res, ContractError::PairConfigNotFound {});

    let res = execute(
        deps.as_mut(),
        env,
        info,
        ExecuteMsg::CreatePair {
            pair_type: PairType::Xyk {},
            asset_infos: asset_infos.clone(),
            init_params: None,
            total_fee_bps: None,
            staking_config: PartialStakeConfig::default(),
        },
    )
    .unwrap();

    assert_eq!(
        res.attributes,
        vec![
            attr("action", "create_pair"),
            attr("pair", format!("{}-{}", asset0000, asset0001))
        ]
    );
    assert_eq!(
        res.messages,
        vec![SubMsg {
            msg: WasmMsg::Instantiate {
                msg: to_json_binary(&PairInstantiateMsg {
                    factory_addr: MOCK_CONTRACT_ADDR.to_string(),
                    asset_infos,
                    token_code_id: msg.token_code_id,
                    init_params: None,
                    staking_config: default_stake_config().to_stake_config(),
                    trading_starts: mock_env().block.time.seconds(),
                    fee_config: pair_config.fee_config,
                    circuit_breaker: None,
                })
                .unwrap(),
                code_id: pair_config.code_id,
                funds: vec![],
                admin: Some(config.unwrap().owner.to_string()),
                label: String::from("Wyndex pair"),
            }
            .into(),
            id: 1,
            gas_limit: None,
            reply_on: ReplyOn::Success,
            payload: Binary::new(vec![])
        }]
    );
}

#[test]
fn register() {
    let mut deps = mock_dependencies(&[]);
    let owner = MockApi::default().addr_make("owner0000");

    let msg = InstantiateMsg {
        pair_configs: vec![PairConfig {
            code_id: 123u64,
            pair_type: PairType::Xyk {},
            fee_config: FeeConfig {
                total_fee_bps: 100,
                protocol_fee_bps: 10,
            },
            is_disabled: false,
        }],
        token_code_id: 123u64,
        fee_address: None,
        owner: owner.to_string(),
        max_referral_commission: Decimal256::one(),
        default_stake_config: default_stake_config(),
        trading_starts: None,
    };

    let env = mock_env();
    let addr0000 = MockApi::default().addr_make("addr0000");
    let info = message_info(&addr0000, &[]);
    let _res = instantiate(deps.as_mut(), env, info, msg).unwrap();

    let asset0000 = MockApi::default().addr_make("asset0000").to_string();
    let asset0001 = MockApi::default().addr_make("asset0001").to_string();
    let asset0002 = MockApi::default().addr_make("asset0002").to_string();
    let asset_infos = vec![
        AssetInfo::Token(asset0000.clone()),
        AssetInfo::Token(asset0001.clone()),
    ];

    let msg = ExecuteMsg::CreatePair {
        pair_type: PairType::Xyk {},
        asset_infos: asset_infos.clone(),
        init_params: None,
        staking_config: PartialStakeConfig::default(),
        total_fee_bps: None,
    };

    let env = mock_env();
    let info = message_info(&owner, &[]);
    let _res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();

    let pair0000 = MockApi::default().addr_make("pair0000");
    let stake0000 = MockApi::default().addr_make("stake0000");
    let liquidity0000 = MockApi::default().addr_make("liquidity0000");
    let pair0_addr = pair0000.to_string();
    let validated_asset_infos: Vec<_> = asset_infos
        .iter()
        .cloned()
        .map(|a| a.validate(&deps.api).unwrap())
        .collect();
    let pair0_info = PairInfo {
        asset_infos: validated_asset_infos.clone(),
        contract_addr: pair0000.clone(),
        staking_addr: stake0000.clone(),
        liquidity_token: liquidity0000.clone(),
        pair_type: PairType::Xyk {},
        fee_config: FeeConfig {
            total_fee_bps: 0,
            protocol_fee_bps: 0,
        },
    };

    let mut deployed_pairs = vec![(&pair0_addr, &pair0_info)];

    // Register an Wyndex pair querier
    deps.querier.with_wyndex_pairs(&deployed_pairs);

    let instantiate_res = MsgInstantiateContractResponse {
        contract_address: pair0000.to_string(),
        data: None,
    };

    let _res = reply::instantiate_pair(deps.as_mut(), mock_env(), instantiate_res.clone()).unwrap();

    let query_res = query(
        deps.as_ref(),
        env,
        QueryMsg::Pair {
            asset_infos: asset_infos.clone(),
        },
    )
    .unwrap();

    let pair_res: PairInfo = from_json(&query_res).unwrap();
    assert_eq!(
        pair_res,
        PairInfo {
            liquidity_token: liquidity0000.clone(),
            contract_addr: pair0000.clone(),
            staking_addr: stake0000.clone(),
            asset_infos: validated_asset_infos.clone(),
            pair_type: PairType::Xyk {},
            fee_config: FeeConfig {
                total_fee_bps: 0,
                protocol_fee_bps: 0,
            },
        }
    );

    // Check pair was registered
    let res = reply::instantiate_pair(deps.as_mut(), mock_env(), instantiate_res).unwrap_err();
    assert_eq!(res, ContractError::PairWasRegistered {});

    // Store one more item to test query pairs
    let asset_infos_2 = vec![
        AssetInfo::Token(asset0000.clone()),
        AssetInfo::Token(asset0002.clone()),
    ];
    let validated_asset_infos_2: Vec<_> = asset_infos_2
        .iter()
        .cloned()
        .map(|a| a.validate(&deps.api).unwrap())
        .collect();

    let msg = ExecuteMsg::CreatePair {
        pair_type: PairType::Xyk {},
        asset_infos: asset_infos_2.clone(),
        init_params: None,
        staking_config: PartialStakeConfig::default(),
        total_fee_bps: None,
    };

    let env = mock_env();
    let info = message_info(&owner, &[]);
    let _res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();

    let pair0001 = MockApi::default().addr_make("pair0001");
    let stake0001 = MockApi::default().addr_make("stake0001");
    let liquidity0001 = MockApi::default().addr_make("liquidity0001");
    let pair1_addr = pair0001.to_string();
    let pair1_info = PairInfo {
        asset_infos: validated_asset_infos_2.clone(),
        contract_addr: pair0001.clone(),
        staking_addr: stake0001.clone(),
        liquidity_token: liquidity0001.clone(),
        pair_type: PairType::Xyk {},
        fee_config: FeeConfig {
            total_fee_bps: 0,
            protocol_fee_bps: 0,
        },
    };

    deployed_pairs.push((&pair1_addr, &pair1_info));

    // Register wyndex pair querier
    deps.querier.with_wyndex_pairs(&deployed_pairs);

    let instantiate_res = MsgInstantiateContractResponse {
        contract_address: pair0001.to_string(),
        data: None,
    };

    let _res = reply::instantiate_pair(deps.as_mut(), mock_env(), instantiate_res).unwrap();

    let query_msg = QueryMsg::Pairs {
        start_after: None,
        limit: None,
    };

    let res = query(deps.as_ref(), env.clone(), query_msg).unwrap();
    let pairs_res: PairsResponse = from_json(&res).unwrap();
    assert_eq!(pairs_res.pairs.len(), 2);
    assert!(pairs_res.pairs.contains(&PairInfo {
        liquidity_token: liquidity0000.clone(),
        contract_addr: pair0000.clone(),
        staking_addr: stake0000.clone(),
        asset_infos: validated_asset_infos.clone(),
        pair_type: PairType::Xyk {},
        fee_config: FeeConfig {
            total_fee_bps: 0,
            protocol_fee_bps: 0,
        },
    }));
    assert!(pairs_res.pairs.contains(&PairInfo {
        liquidity_token: liquidity0001.clone(),
        contract_addr: pair0001.clone(),
        staking_addr: stake0001.clone(),
        asset_infos: validated_asset_infos_2.clone(),
        pair_type: PairType::Xyk {},
        fee_config: FeeConfig {
            total_fee_bps: 0,
            protocol_fee_bps: 0,
        },
    }));

    // Determine which pair sorts first in PAIRS storage to test limit and start_after
    let (first_pair_info, second_pair_info) = {
        let first = &pairs_res.pairs[0];
        if first.contract_addr == pair0000 {
            (
                PairInfo {
                    liquidity_token: liquidity0000.clone(),
                    contract_addr: pair0000.clone(),
                    staking_addr: stake0000.clone(),
                    asset_infos: validated_asset_infos.clone(),
                    pair_type: PairType::Xyk {},
                    fee_config: FeeConfig {
                        total_fee_bps: 0,
                        protocol_fee_bps: 0,
                    },
                },
                PairInfo {
                    liquidity_token: liquidity0001.clone(),
                    contract_addr: pair0001.clone(),
                    staking_addr: stake0001.clone(),
                    asset_infos: validated_asset_infos_2.clone(),
                    pair_type: PairType::Xyk {},
                    fee_config: FeeConfig {
                        total_fee_bps: 0,
                        protocol_fee_bps: 0,
                    },
                },
            )
        } else {
            (
                PairInfo {
                    liquidity_token: liquidity0001.clone(),
                    contract_addr: pair0001.clone(),
                    staking_addr: stake0001.clone(),
                    asset_infos: validated_asset_infos_2.clone(),
                    pair_type: PairType::Xyk {},
                    fee_config: FeeConfig {
                        total_fee_bps: 0,
                        protocol_fee_bps: 0,
                    },
                },
                PairInfo {
                    liquidity_token: liquidity0000.clone(),
                    contract_addr: pair0000.clone(),
                    staking_addr: stake0000.clone(),
                    asset_infos: validated_asset_infos.clone(),
                    pair_type: PairType::Xyk {},
                    fee_config: FeeConfig {
                        total_fee_bps: 0,
                        protocol_fee_bps: 0,
                    },
                },
            )
        }
    };
    let first_asset_infos_unvalidated: Vec<AssetInfo> = first_pair_info
        .asset_infos
        .iter()
        .map(|a| match a {
            wyndex::asset::AssetInfoValidated::Token(addr) => AssetInfo::Token(addr.to_string()),
            wyndex::asset::AssetInfoValidated::Native(denom) => AssetInfo::Native(denom.clone()),
        })
        .collect();

    let query_msg = QueryMsg::Pairs {
        start_after: None,
        limit: Some(1),
    };

    let res = query(deps.as_ref(), env.clone(), query_msg).unwrap();
    let pairs_res: PairsResponse = from_json(&res).unwrap();
    assert_eq!(pairs_res.pairs, vec![first_pair_info]);

    let query_msg = QueryMsg::Pairs {
        start_after: Some(first_asset_infos_unvalidated),
        limit: None,
    };

    let res = query(deps.as_ref(), env, query_msg).unwrap();
    let pairs_res: PairsResponse = from_json(&res).unwrap();
    assert_eq!(pairs_res.pairs, vec![second_pair_info]);

    // Deregister from wrong acc
    let env = mock_env();
    let info = message_info(&Addr::unchecked("wrong_addr0000"), &[]);
    let res = execute(
        deps.as_mut(),
        env,
        info,
        ExecuteMsg::Deregister {
            asset_infos: asset_infos_2.clone(),
        },
    )
    .unwrap_err();

    assert_eq!(res, ContractError::Unauthorized {});

    // Proper deregister
    let env = mock_env();
    let info = message_info(&owner, &[]);
    let res = execute(
        deps.as_mut(),
        env.clone(),
        info,
        ExecuteMsg::Deregister {
            asset_infos: asset_infos_2,
        },
    )
    .unwrap();

    assert_eq!(res.attributes[0], attr("action", "deregister"));

    let query_msg = QueryMsg::Pairs {
        start_after: None,
        limit: None,
    };

    let res = query(deps.as_ref(), env, query_msg).unwrap();
    let pairs_res: PairsResponse = from_json(&res).unwrap();
    assert_eq!(
        pairs_res.pairs,
        vec![PairInfo {
            liquidity_token: liquidity0000,
            contract_addr: pair0000,
            staking_addr: stake0000,
            asset_infos: validated_asset_infos,
            pair_type: PairType::Xyk {},
            fee_config: FeeConfig {
                total_fee_bps: 0,
                protocol_fee_bps: 0,
            },
        },]
    );
}
