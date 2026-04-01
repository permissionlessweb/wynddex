use anyhow::Result as AnyResult;
use cosmwasm_std::{coin, to_json_binary, Addr, Coin, Decimal256, Uint128, Uint256};
use cw20::{BalanceResponse, Cw20ExecuteMsg, Cw20QueryMsg, MinterResponse};
use cw20_base::msg::InstantiateMsg as Cw20BaseInstantiateMsg;
use cw_multi_test::{App, AppResponse, BankSudo, ContractWrapper, Executor, SudoMsg};
use wyndex_test_helpers::TestAccounts;

use crate::msg::{
    ExecuteMsg, InstantiateMsg, QueryMsg, SimulateSwapOperationsResponse, SwapOperation,
};
use wyndex::asset::{Asset, AssetInfo};
use wyndex::factory::{
    DefaultStakeConfig, ExecuteMsg as FactoryExecuteMsg, InstantiateMsg as FactoryInstantiateMsg,
    PairConfig, PairType, QueryMsg as FactoryQueryMsg,
};
use wyndex::fee_config::FeeConfig;
use wyndex::pair::{ExecuteMsg as PairExecuteMsg, PairInfo};

const SECONDS_PER_DAY: u64 = 60 * 60 * 24;

fn store_multi_hop(app: &mut App) -> u64 {
    let contract = Box::new(ContractWrapper::new_with_empty(
        crate::contract::execute,
        crate::contract::instantiate,
        crate::contract::query,
    ));

    app.store_code(contract)
}

fn store_factory(app: &mut App) -> u64 {
    let contract = Box::new(
        ContractWrapper::new_with_empty(
            wyndex_factory::contract::execute,
            wyndex_factory::contract::instantiate,
            wyndex_factory::contract::query,
        )
        .with_reply_empty(wyndex_factory::contract::reply),
    );

    app.store_code(contract)
}

fn store_pair(app: &mut App) -> u64 {
    let contract = Box::new(
        ContractWrapper::new_with_empty(
            wyndex_pair::contract::execute,
            wyndex_pair::contract::instantiate,
            wyndex_pair::contract::query,
        )
        .with_reply_empty(wyndex_pair::contract::reply),
    );

    app.store_code(contract)
}

fn store_cw20(app: &mut App) -> u64 {
    let contract = Box::new(ContractWrapper::new(
        cw20_base::contract::execute,
        cw20_base::contract::instantiate,
        cw20_base::contract::query,
    ));

    app.store_code(contract)
}

fn store_staking(app: &mut App) -> u64 {
    let contract = Box::new(ContractWrapper::new(
        wyndex_stake::contract::execute,
        wyndex_stake::contract::instantiate,
        wyndex_stake::contract::query,
    ));

    app.store_code(contract)
}

#[derive(Debug)]
pub struct SuiteBuilder {
    funds: Vec<(Addr, Vec<Coin>)>,
    max_referral_commission: Decimal256,
    stake_config: DefaultStakeConfig,
    total_fee_bps: u16,
    protocol_fee_bps: u16,
}

impl SuiteBuilder {
    pub fn new() -> Self {
        Self {
            funds: vec![],
            max_referral_commission: Decimal256::one(),
            total_fee_bps: 0,
            protocol_fee_bps: 0,
            stake_config: DefaultStakeConfig {
                staking_code_id: 0, // will be set in build()
                tokens_per_power: Uint256::new(1000),
                min_bond: Uint256::new(1000),
                unbonding_periods: vec![
                    SECONDS_PER_DAY * 7,
                    SECONDS_PER_DAY * 14,
                    SECONDS_PER_DAY * 21,
                ],
                max_distributions: 6,
                converter: None,
            },
        }
    }

    pub fn with_fees(mut self, total_fee_bps: u16, protocol_fee_bps: u16) -> Self {
        self.total_fee_bps = total_fee_bps;
        self.protocol_fee_bps = protocol_fee_bps;
        self
    }

    pub fn with_funds(mut self, addr: &Addr, funds: &[Coin]) -> Self {
        self.funds.push((addr.clone(), funds.into()));
        self
    }

    pub fn with_max_referral_commission(mut self, max: Decimal256) -> Self {
        self.max_referral_commission = max;
        self
    }

    #[track_caller]
    pub fn build(self) -> Suite {
        let mut app = App::default();
        let a = TestAccounts::new(&app.api());
        let owner = a.owner.clone();

        let cw20_code_id = store_cw20(&mut app);
        let pair_code_id = store_pair(&mut app);
        let factory_code_id = store_factory(&mut app);
        let staking_code_id = store_staking(&mut app);
        let factory = app
            .instantiate_contract(
                factory_code_id,
                owner.clone(),
                &FactoryInstantiateMsg {
                    pair_configs: vec![
                        PairConfig {
                            code_id: pair_code_id,
                            pair_type: PairType::Xyk {},
                            fee_config: FeeConfig {
                                total_fee_bps: self.total_fee_bps,
                                protocol_fee_bps: self.protocol_fee_bps,
                            },
                            is_disabled: false,
                        },
                        PairConfig {
                            code_id: pair_code_id,
                            pair_type: PairType::Stable {},
                            fee_config: FeeConfig {
                                total_fee_bps: self.total_fee_bps,
                                protocol_fee_bps: self.protocol_fee_bps,
                            },
                            is_disabled: false,
                        },
                    ],
                    token_code_id: cw20_code_id,
                    fee_address: None,
                    owner: owner.to_string(),
                    max_referral_commission: self.max_referral_commission,
                    default_stake_config: DefaultStakeConfig {
                        staking_code_id,
                        ..self.stake_config
                    },
                    trading_starts: None,
                },
                &[],
                "Wyndex Factory",
                None,
            )
            .unwrap();

        let multi_hop_code_id = store_multi_hop(&mut app);
        let multi_hop = app
            .instantiate_contract(
                multi_hop_code_id,
                owner.clone(),
                &InstantiateMsg {
                    wyndex_factory: factory.to_string(),
                },
                &[],
                "Wyndex Multi Hop",
                None,
            )
            .unwrap();

        let funds = self.funds;
        app.init_modules(|router, _, storage| -> AnyResult<()> {
            for (addr, coin) in funds {
                router
                    .bank
                    .init_balance(storage, &addr, coin)
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
            }
            Ok(())
        })
        .unwrap();

        Suite {
            a: TestAccounts::new(App::default().api()),
            app,
            factory,
            multi_hop,
            cw20_code_id,
        }
    }
}

pub struct Suite {
    pub a: TestAccounts,
    pub app: App,
    pub factory: Addr,
    multi_hop: Addr,
    cw20_code_id: u64,
}

impl Suite {
    fn create_pair(
        &mut self,
        sender: &Addr,
        pair_type: PairType,
        tokens: [AssetInfo; 2],
    ) -> AnyResult<Addr> {
        self.app
            .execute_contract(
                sender.to_owned(),
                self.factory.clone(),
                &FactoryExecuteMsg::CreatePair {
                    pair_type,
                    asset_infos: tokens.to_vec(),
                    init_params: None,
                    staking_config: Default::default(),
                    total_fee_bps: None,
                },
                &[],
            )
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        let factory = self.factory.clone();
        let res: PairInfo = self
            .app
            .wrap()
            .query_wasm_smart(
                factory,
                &FactoryQueryMsg::Pair {
                    asset_infos: tokens.to_vec(),
                },
            )
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok(res.contract_addr)
    }

    fn provide_liquidity(
        &mut self,
        owner: &Addr,
        pair: &Addr,
        assets: [Asset; 2],
        send_funds: &[Coin],
    ) -> AnyResult<AppResponse> {
        self.app
            .execute_contract(
                owner.to_owned(),
                pair.clone(),
                &PairExecuteMsg::ProvideLiquidity {
                    assets: assets.to_vec(),
                    slippage_tolerance: None,
                    receiver: None,
                },
                send_funds,
            )
            .map_err(|e| anyhow::anyhow!("{e}"))
    }

    fn increase_allowance(
        &mut self,
        owner: &Addr,
        contract: &Addr,
        spender: &str,
        amount: u128,
    ) -> AnyResult<AppResponse> {
        self.app
            .execute_contract(
                owner.to_owned(),
                contract.clone(),
                &Cw20ExecuteMsg::IncreaseAllowance {
                    spender: spender.to_owned(),
                    amount: amount.into(),
                    expires: None,
                },
                &[],
            )
            .map_err(|e| anyhow::anyhow!("{e}"))
    }

    /// Create LP for provided assets and provides some liquidity to them.
    /// Requirement: if using native token provide coins to sent as last argument
    pub fn create_pair_and_provide_liquidity(
        &mut self,
        pair_type: PairType,
        first_asset: (AssetInfo, u128),
        second_asset: (AssetInfo, u128),
        native_tokens: Vec<Coin>,
    ) -> AnyResult<Addr> {
        let owner = self.a.owner.clone();
        let whale = self.a.whale.clone();

        let pair = self.create_pair(
            &owner,
            pair_type,
            [first_asset.0.clone(), second_asset.0.clone()],
        )?;

        match first_asset.0.clone() {
            AssetInfo::Token(addr) => {
                // Mint some initial balances for whale user
                self.mint_cw20(&owner, &Addr::unchecked(&addr), first_asset.1, &whale)
                    .unwrap();
                // Increases allowances for given LP contracts in order to provide liquidity to pool
                self.increase_allowance(
                    &whale,
                    &Addr::unchecked(&addr),
                    pair.as_str(),
                    first_asset.1,
                )
                .unwrap();
            }
            AssetInfo::Native(denom) => {
                self.app
                    .sudo(SudoMsg::Bank(BankSudo::Mint {
                        to_address: whale.to_string(),
                        amount: vec![coin(first_asset.1, denom)],
                    }))
                    .unwrap();
            }
        };
        match second_asset.0.clone() {
            AssetInfo::Token(addr) => {
                // Mint some initial balances for whale user
                self.mint_cw20(&owner, &Addr::unchecked(&addr), second_asset.1, &whale)
                    .unwrap();
                // Increases allowances for given LP contracts in order to provide liquidity to pool
                self.increase_allowance(
                    &whale,
                    &Addr::unchecked(&addr),
                    pair.as_str(),
                    second_asset.1,
                )
                .unwrap();
            }
            AssetInfo::Native(denom) => {
                self.app
                    .sudo(SudoMsg::Bank(BankSudo::Mint {
                        to_address: whale.to_string(),
                        amount: vec![coin(second_asset.1, denom)],
                    }))
                    .unwrap();
            }
        };

        self.provide_liquidity(
            &whale,
            &pair,
            [
                Asset {
                    info: first_asset.0,
                    amount: first_asset.1.into(),
                },
                Asset {
                    info: second_asset.0,
                    amount: second_asset.1.into(),
                },
            ],
            &native_tokens, // for native token you need to transfer tokens manually
        )
        .unwrap();

        Ok(pair)
    }

    pub fn instantiate_token(&mut self, owner: &Addr, token: &str) -> Addr {
        self.app
            .instantiate_contract(
                self.cw20_code_id,
                owner.to_owned(),
                &Cw20BaseInstantiateMsg {
                    name: token.to_owned(),
                    symbol: token.to_owned(),
                    decimals: 6,
                    initial_balances: vec![],
                    mint: Some(MinterResponse {
                        minter: owner.to_string(),
                        cap: None,
                    }),
                    marketing: None,
                },
                &[],
                token,
                None,
            )
            .unwrap()
    }

    pub fn mint_cw20(
        &mut self,
        owner: &Addr,
        token: &Addr,
        amount: u128,
        recipient: &Addr,
    ) -> AnyResult<AppResponse> {
        self.app
            .execute_contract(
                owner.to_owned(),
                token.clone(),
                &Cw20ExecuteMsg::Mint {
                    recipient: recipient.to_string(),
                    amount: amount.into(),
                },
                &[],
            )
            .map_err(|e| anyhow::anyhow!("{e}"))
    }

    pub fn swap_operations(
        &mut self,
        sender: &Addr,
        amount: Coin,
        operations: Vec<SwapOperation>,
    ) -> AnyResult<AppResponse> {
        self.swap_operations_ref(sender, amount, operations, None, None)
    }

    pub fn swap_operations_ref(
        &mut self,
        sender: &Addr,
        amount: Coin,
        operations: Vec<SwapOperation>,
        referral_address: impl Into<Option<String>>,
        referral_commission: impl Into<Option<Decimal256>>,
    ) -> AnyResult<AppResponse> {
        self.app
            .execute_contract(
                sender.to_owned(),
                self.multi_hop.clone(),
                &ExecuteMsg::ExecuteSwapOperations {
                    operations,
                    minimum_receive: None,
                    receiver: None,
                    max_spread: None,
                    referral_address: referral_address.into(),
                    referral_commission: referral_commission.into(),
                },
                &[amount],
            )
            .map_err(|e| anyhow::anyhow!("{e}"))
    }

    pub fn swap_operations_cw20(
        &mut self,
        sender: &Addr,
        token_in: &Addr,
        amount: u128,
        operations: Vec<SwapOperation>,
    ) -> AnyResult<AppResponse> {
        self.swap_operations_cw20_ref(sender, token_in, amount, operations, None, None)
    }

    pub fn swap_operations_cw20_ref(
        &mut self,
        sender: &Addr,
        token_in: &Addr,
        amount: u128,
        operations: Vec<SwapOperation>,
        referral_address: impl Into<Option<String>>,
        referral_commission: impl Into<Option<Decimal256>>,
    ) -> AnyResult<AppResponse> {
        self.app
            .execute_contract(
                sender.to_owned(),
                token_in.clone(),
                &Cw20ExecuteMsg::Send {
                    contract: self.multi_hop.to_string(),
                    amount: amount.into(),
                    msg: to_json_binary(&ExecuteMsg::ExecuteSwapOperations {
                        operations,
                        minimum_receive: None,
                        receiver: None,
                        max_spread: None,
                        referral_address: referral_address.into(),
                        referral_commission: referral_commission.into(),
                    })
                    .unwrap(),
                },
                &[],
            )
            .map_err(|e| anyhow::anyhow!("{e}"))
    }

    pub fn assert_minimum_receive(
        &mut self,
        receiver: &Addr,
        asset_info: AssetInfo,
        minimum_receive: impl Into<Uint256>,
    ) -> AnyResult<AppResponse> {
        self.app
            .execute_contract(
                Addr::unchecked(receiver),
                self.multi_hop.clone(),
                &ExecuteMsg::AssertMinimumReceive {
                    asset_info,
                    prev_balance: Uint256::zero(),
                    minimum_receive: minimum_receive.into(),
                    receiver: receiver.into(),
                },
                &[],
            )
            .map_err(|e| anyhow::anyhow!("{e}"))
    }

    pub fn query_balance(&self, sender: &Addr, denom: &str) -> AnyResult<u128> {
        let amount = self
            .app
            .wrap()
            .query_balance(sender, denom)
            .map_err(|e| anyhow::anyhow!("{e}"))?
            .amount;
        Ok(Uint128::try_from(amount).unwrap().u128())
    }

    pub fn query_cw20_balance(&self, sender: &Addr, address: &Addr) -> AnyResult<u128> {
        let balance: BalanceResponse = self
            .app
            .wrap()
            .query_wasm_smart(
                address,
                &Cw20QueryMsg::Balance {
                    address: sender.to_string(),
                },
            )
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok(Uint128::try_from(balance.balance).unwrap().u128())
    }

    pub fn query_simulate_swap_operations(
        &self,
        offer_amount: impl Into<Uint256>,
        operations: Vec<SwapOperation>,
    ) -> AnyResult<SimulateSwapOperationsResponse> {
        let res: SimulateSwapOperationsResponse = self
            .app
            .wrap()
            .query_wasm_smart(
                self.multi_hop.clone(),
                &QueryMsg::SimulateSwapOperations {
                    offer_amount: offer_amount.into(),
                    operations,
                    referral: false,
                    referral_commission: None,
                },
            )
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok(res)
    }

    pub fn query_simulate_swap_operations_ref(
        &self,
        offer_amount: impl Into<Uint256>,
        operations: Vec<SwapOperation>,
        referral_commission: impl Into<Option<Decimal256>>,
    ) -> AnyResult<SimulateSwapOperationsResponse> {
        let res: SimulateSwapOperationsResponse = self
            .app
            .wrap()
            .query_wasm_smart(
                self.multi_hop.clone(),
                &QueryMsg::SimulateSwapOperations {
                    offer_amount: offer_amount.into(),
                    operations,
                    referral: true,
                    referral_commission: referral_commission.into(),
                },
            )
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok(res)
    }

    pub fn query_simulate_reverse_swap_operations(
        &self,
        ask_amount: impl Into<Uint256>,
        operations: Vec<SwapOperation>,
    ) -> AnyResult<SimulateSwapOperationsResponse> {
        let res: SimulateSwapOperationsResponse = self
            .app
            .wrap()
            .query_wasm_smart(
                self.multi_hop.clone(),
                &QueryMsg::SimulateReverseSwapOperations {
                    ask_amount: ask_amount.into(),
                    operations,
                    referral: false,
                    referral_commission: None,
                },
            )
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok(res)
    }

    pub fn query_simulate_reverse_swap_operations_ref(
        &self,
        ask_amount: impl Into<Uint256>,
        operations: Vec<SwapOperation>,
        referral_commission: impl Into<Option<Decimal256>>,
    ) -> AnyResult<SimulateSwapOperationsResponse> {
        let res: SimulateSwapOperationsResponse = self
            .app
            .wrap()
            .query_wasm_smart(
                self.multi_hop.clone(),
                &QueryMsg::SimulateReverseSwapOperations {
                    ask_amount: ask_amount.into(),
                    operations,
                    referral: true,
                    referral_commission: referral_commission.into(),
                },
            )
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok(res)
    }
}
