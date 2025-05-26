use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Serialize, Deserialize,Clone)]
pub struct QuoteResp {
    pub contextSlot: u64,
    pub inAmount: String,
    pub inputMint: String,
    pub mostReliableAmmsQuoteReport: MostReliableAmmsQuoteReport,
    pub otherAmountThreshold: String,
    pub otherRoutePlans: Option<()>,  // 根据实际null类型处理
    pub outAmount: String,
    pub outputMint: String,
    pub platformFee: Option<()>,      // 根据实际null类型处理
    pub priceImpactPct: String,
    pub routePlan: Vec<RoutePlan>,
    pub simplerRouteUsed: bool,
    pub slippageBps: u32,            // 根据实际取值范围选择类型
    pub swapMode: String,
    pub swapUsdValue: String,
    pub timeTaken: f64,
    pub useIncurredSlippageForQuoting: Option<()>, // 根据实际null类型处理
}

#[derive(Debug, Serialize, Deserialize,Clone)]
pub struct MostReliableAmmsQuoteReport {
    pub info: HashMap<String, String>,
}

#[derive(Debug, Serialize, Deserialize,Clone)]
pub struct RoutePlan {
    pub percent: u8,
    pub swapInfo: SwapInfo,
}

#[derive(Debug, Serialize, Deserialize,Clone)]
pub struct SwapInfo {
    pub ammKey: String,
    pub feeAmount: String,
    pub feeMint: String,
    pub inAmount: String,
    pub inputMint: String,
    pub label: String,
    pub outAmount: String,
    pub outputMint: String,
}

#[derive(Debug,Serialize,Deserialize,Clone)]
pub struct Quote_param <'a>{
    pub inputMint:String,
    pub outputMint:String,
    pub amount:&'a str,
    pub onlyDirectRoutes:bool,
    pub slippageBps: u8,
    pub maxAccounts: u8
}