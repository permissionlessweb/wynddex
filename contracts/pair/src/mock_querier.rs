use cosmwasm_std::testing::{MockApi, MockQuerier, MockStorage, MOCK_CONTRACT_ADDR};
use cosmwasm_std::{
    from_json, to_json_binary, Addr, Coin, Decimal256, Empty, OwnedDeps, Querier, QuerierResult,
    QueryRequest, SystemError, SystemResult, Uint128, WasmQuery,
};
use std::collections::HashMap;
use wyndex_test_helpers::TestAccounts;

use cw20::{BalanceResponse, Cw20QueryMsg, TokenInfoResponse};
use wyndex::factory::{
    ConfigResponse, FeeInfoResponse,
    QueryMsg::{Config, FeeInfo},
};

/// mock_dependencies is a drop-in replacement for cosmwasm_std::testing::mock_dependencies.
/// This uses the Wyndex CustomQuerier.
pub fn mock_dependencies(
    contract_balance: &[Coin],
) -> OwnedDeps<MockStorage, MockApi, WasmMockQuerier> {
    let custom_querier: WasmMockQuerier =
        WasmMockQuerier::new(MockQuerier::new(&[(MOCK_CONTRACT_ADDR, contract_balance)]));

    OwnedDeps {
        storage: MockStorage::default(),
        api: MockApi::default(),
        querier: custom_querier,
        custom_query_type: Default::default(),
    }
}

pub struct WasmMockQuerier {
    base: MockQuerier<Empty>,
    token_querier: TokenQuerier,
}

#[derive(Clone, Default)]
pub struct TokenQuerier {
    // This lets us iterate over all pairs that match the first string
    balances: HashMap<String, HashMap<String, Uint128>>,
}

impl TokenQuerier {
    pub fn new(balances: &[(&String, &[(&String, &Uint128)])]) -> Self {
        TokenQuerier {
            balances: balances_to_map(balances),
        }
    }
}

pub(crate) fn balances_to_map(
    balances: &[(&String, &[(&String, &Uint128)])],
) -> HashMap<String, HashMap<String, Uint128>> {
    let mut balances_map: HashMap<String, HashMap<String, Uint128>> = HashMap::new();
    for (contract_addr, balances) in balances.iter() {
        let mut contract_balances_map: HashMap<String, Uint128> = HashMap::new();
        for (addr, balance) in balances.iter() {
            contract_balances_map.insert(addr.to_string(), **balance);
        }

        balances_map.insert(contract_addr.to_string(), contract_balances_map);
    }
    balances_map
}

impl Querier for WasmMockQuerier {
    fn raw_query(&self, bin_request: &[u8]) -> QuerierResult {
        // MockQuerier doesn't support Custom, so we ignore it completely
        let request: QueryRequest<Empty> = match from_json(bin_request) {
            Ok(v) => v,
            Err(e) => {
                return SystemResult::Err(SystemError::InvalidRequest {
                    error: format!("Parsing query request: {}", e),
                    request: bin_request.into(),
                })
            }
        };
        self.handle_query(&request)
    }
}

impl WasmMockQuerier {
    pub fn handle_query(&self, request: &QueryRequest<Empty>) -> QuerierResult {
        let a = TestAccounts::new(&MockApi::default());
        let factory_addr = a.beneficiary;
        let fee_address = a.fee_receiver;
        let owner = a.owner;
        match &request {
            QueryRequest::Wasm(WasmQuery::Smart { contract_addr, msg }) => {
                if contract_addr == factory_addr.as_str() {
                    match from_json(msg).unwrap() {
                        FeeInfo { .. } => SystemResult::Ok(
                            to_json_binary(&FeeInfoResponse {
                                fee_address: Some(fee_address),
                                total_fee_bps: 30,
                                protocol_fee_bps: 1660,
                            })
                            .into(),
                        ),
                        Config {} => SystemResult::Ok(
                            to_json_binary(&ConfigResponse {
                                owner,
                                pair_configs: vec![],
                                token_code_id: 0,
                                fee_address: Some(fee_address),
                                max_referral_commission: Decimal256::one(),
                                only_owner_can_create_pairs: true,
                                trading_starts: None,
                            })
                            .into(),
                        ),
                        _ => panic!("DO NOT ENTER HERE"),
                    }
                } else {
                    match from_json(msg).unwrap() {
                        Cw20QueryMsg::TokenInfo {} => {
                            let balances: &HashMap<String, Uint128> =
                                match self.token_querier.balances.get(contract_addr) {
                                    Some(balances) => balances,
                                    None => {
                                        return SystemResult::Err(SystemError::Unknown {});
                                    }
                                };

                            let mut total_supply = Uint128::zero();

                            for balance in balances {
                                total_supply += *balance.1;
                            }

                            SystemResult::Ok(
                                to_json_binary(&TokenInfoResponse {
                                    name: "mAPPL".to_string(),
                                    symbol: "mAPPL".to_string(),
                                    decimals: 6,
                                    total_supply: total_supply.into(),
                                })
                                .into(),
                            )
                        }
                        Cw20QueryMsg::Balance { address } => {
                            let balances: &HashMap<String, Uint128> =
                                match self.token_querier.balances.get(contract_addr) {
                                    Some(balances) => balances,
                                    None => {
                                        return SystemResult::Err(SystemError::Unknown {});
                                    }
                                };

                            let balance = match balances.get(&address) {
                                Some(v) => v,
                                None => {
                                    return SystemResult::Err(SystemError::Unknown {});
                                }
                            };

                            SystemResult::Ok(
                                to_json_binary(&BalanceResponse {
                                    balance: (*balance).into(),
                                })
                                .into(),
                            )
                        }
                        _ => panic!("DO NOT ENTER HERE"),
                    }
                }
            }
            QueryRequest::Wasm(WasmQuery::Raw { contract_addr, .. }) => {
                if contract_addr == factory_addr.as_str() {
                    SystemResult::Ok(to_json_binary(&Vec::<Addr>::new()).into())
                } else {
                    panic!("DO NOT ENTER HERE");
                }
            }
            _ => self.base.handle_query(request),
        }
    }
}

impl WasmMockQuerier {
    pub fn new(base: MockQuerier<Empty>) -> Self {
        WasmMockQuerier {
            base,
            token_querier: TokenQuerier::default(),
        }
    }

    // Configure the mint whitelist mock querier
    pub fn with_token_balances(&mut self, balances: &[(&String, &[(&String, &Uint128)])]) {
        self.token_querier = TokenQuerier::new(balances);
    }

    pub fn with_balance(&mut self, balances: &[(&String, &[Coin])]) {
        for (addr, balance) in balances {
            self.base
                .bank
                .update_balance(addr.to_string(), balance.to_vec());
        }
    }
}
