use std::{env, str::FromStr, sync::Arc};
use dotenv::dotenv;
use reqwest::Client as HttpClient;
use serde::{Deserialize, Serialize};
use solana_client::{
    rpc_client::RpcClient
    //rpc_config::RpcSendTransactionConfig,
    //rpc_response::RpcResult,
};
use solana_sdk::{
    pubkey::Pubkey,
    signature::{Keypair, Signer, Signature},
    transaction::VersionedTransaction,
    message::{v0::Message as MessageV0, VersionedMessage},
    instruction::{Instruction, AccountMeta},
    compute_budget,
    system_instruction,
   
    //hash::Hash,
    //signer::keypair::Keypair,
};
use solana_program::address;

use base64::{engine::general_purpose, Engine as _};
use bs58::{self, decode::Error as Bs58Error};
use serde_json::{json, Value};

const QUOTE_URL: &str = "https://api.jup.ag/swap/v1/quote";
const SWAP_URL: &str = "https://lite-api.jup.ag/swap/v1/swap-instructions";
const JITO_TIP_ACCOUNT: &str = "96gYZGLnJYVFmbjzopPSU6QiEV5fGqZNyN9nmNhvrZU5";
const WSOL_MINT: &str = "So11111111111111111111111111111111111111112";
const USDC_MINT: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";

#[derive(Debug, Serialize, Deserialize)]
struct JupiterQuote {
    input_mint: String,
    output_mint: String,
    in_amount: u64,
    out_amount: u64,
    route_plan: Vec<RouteStep>,
    context_slot: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct RouteStep {
    swap_info: SwapInfo,
}

#[derive(Debug, Serialize, Deserialize)]
struct SwapInfo {
    amm_key: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct SwapInstructions {
    compute_unit_limit: u32,
    setup_instructions: Vec<JupiterInstruction>,
    swap_instruction: JupiterInstruction,
    cleanup_instruction: Option<JupiterInstruction>,
    address_lookup_table_addresses: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct JupiterInstruction {
    program_id: String,
    accounts: Vec<AccountMetaData>,
    data: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct AccountMetaData {
    pubkey: String,
    is_signer: bool,
    is_writable: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenv().ok();
    let payer = Keypair::from_base58_string(&env::var("SECRET_KEY")?);
    let rpc_url = "https://mainnet.helius-rpc.com/?api-key=77a3d694-a103-48d1-9c07-fa80651d5390";
    let client = Arc::new(RpcClient::new(rpc_url));

    loop {
        match run(client.clone(), &payer).await {
            Ok(_) => {}
            Err(e) => eprintln!("Error: {:?}", e),
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
}

async fn run(client: Arc<RpcClient>, payer: &Keypair) -> Result<(), Box<dyn std::error::Error>> {
    // 获取报价
    let (merged_quote, diff) = get_merged_quote().await?;
    if diff <= 1000 {
        return Ok(());
    }

    // 获取交换指令
    let swap_instructions = get_swap_instructions(&merged_quote, payer.pubkey()).await?;
    
    // 构建交易
    let transaction = build_transaction(&client, payer, swap_instructions).await?;
    
    // 发送交易
    send_transaction(transaction).await?;
    
    Ok(())
}

async fn get_merged_quote() -> Result<(Value, i64), Box<dyn std::error::Error>> {
    let http = HttpClient::new();
    
    // 第一次报价 WSOL → USDC
    let quote0: JupiterQuote = http.get(QUOTE_URL)
        .query(&[
            ("inputMint", WSOL_MINT),
            ("outputMint", USDC_MINT),
            ("amount", "1000000"),
            ("slippageBps", "100")
        ])
        .send().await?
        .json().await?;

    // 第二次报价 USDC → WSOL
    let quote1: JupiterQuote = http.get(QUOTE_URL)
        .query(&[
            ("inputMint", USDC_MINT),
            ("outputMint", WSOL_MINT),
            ("amount", &quote0.out_amount.to_string()),
            ("slippageBps", "20")
        ])
        .send().await?
        .json().await?;

    // 计算套利空间
    let diff = quote1.out_amount as i64 - 1000000;
    if diff <= 1000 {
        return Ok((json!({}), diff));
    }

    // 合并报价结果
    let merged_quote = json!({
        "inputMint": WSOL_MINT,
        "outputMint": USDC_MINT,
        "inAmount": 1000000,
        "outAmount": quote0.out_amount + 1000,
        "contextSlot": quote0.context_slot,
        "routePlan": quote0.route_plan,
        "prioritizationFeeLamports": {
            "priorityLevelWithMaxLamports": {
                "maxLamports": 10000000,
                "priorityLevel": "veryHigh"
            }
        }
    });

    Ok((merged_quote, diff))
}

async fn get_swap_instructions(quote: &Value, payer: Pubkey) -> Result<SwapInstructions, Box<dyn std::error::Error>> {
    let http = HttpClient::new();
    
    let swap_data = json!({
        "userPublicKey": payer.to_string(),
        "quoteResponse": quote,
        "wrapAndUnwrapSol": true,
        "dynamicComputeUnitLimit": true,
    });

    let instructions: SwapInstructions = http.post(SWAP_URL)
        .json(&swap_data)
        .send().await?
        .json().await?;

    Ok(instructions)
}

async fn build_transaction(
    client: &RpcClient,
    payer: &Keypair,
    instructions: SwapInstructions,
) -> Result<VersionedTransaction, Box<dyn std::error::Error>> {
    // 转换指令
    let mut ixs = vec![
        compute_budget::ComputeBudgetInstruction::set_compute_unit_limit(instructions.compute_unit_limit),
    ];

    // 添加设置指令
    for setup in instructions.setup_instructions {
        ixs.push(convert_jupiter_instruction(setup)?);
    }

    // 添加交换指令
    ixs.push(convert_jupiter_instruction(instructions.swap_instruction)?);

    // 添加清理指令
    if let Some(cleanup) = instructions.cleanup_instruction {
        ixs.push(convert_jupiter_instruction(cleanup)?);
    }

    // 添加Jito小费
    let tip_ix = system_instruction::transfer(
        &payer.pubkey(),
        &Pubkey::from_str(JITO_TIP_ACCOUNT)?,
        1000,
    );
    ixs.push(tip_ix);

    // 获取地址查找表
    let alt_accounts:Vec<AddressLookupTableAccount> = get_address_lookup_tables(client, instructions.address_lookup_table_addresses).await?;

    // 构建版本化消息
    let blockhash = client.get_latest_blockhash()?;
    let message = MessageV0::try_compile(
        &payer.pubkey(),
        &ixs,
        alt_accounts,
        blockhash,
    )?;

    // 签名交易
    let transaction = VersionedTransaction::try_new(
        VersionedMessage::V0(message),
        &[payer]
    )?;

    Ok(transaction)
}

async fn get_address_lookup_tables(
    client: &RpcClient,
    addresses: Vec<String>,
) -> Result<Vec<AddressLookupTableAccount>, Box<dyn std::error::Error>> {
    let mut alt_accounts = Vec::new();
    
    for addr in addresses {
        let pubkey = Pubkey::from_str(&addr)?;
        let account = client.get_account(&pubkey)?;
        
        let lookup_table = bincode::deserialize::<AddressLookupTableAccount>(&account.data)?;
        alt_accounts.push(lookup_table);
    }
    
    Ok(alt_accounts)
}

fn convert_jupiter_instruction(jupiter_ix: JupiterInstruction) -> Result<Instruction, Bs58Error> {
    let program_id = Pubkey::from_str(&jupiter_ix.program_id);
    let accounts = jupiter_ix.accounts.into_iter().map(|meta| {
        AccountMeta {
            pubkey: Pubkey::from_str(&meta.pubkey).unwrap(),
            is_signer: meta.is_signer,
            is_writable: meta.is_writable,
        }
    }).collect();
    
    let data = general_purpose::STANDARD.decode(&jupiter_ix.data);
    
    Ok(Instruction {
        program_id,
        accounts,
        data,
    })
}

async fn send_transaction(tx: VersionedTransaction) -> Result<Signature, Box<dyn std::error::Error>> {
    let http = HttpClient::new();
    
    // 序列化交易
    let serialized = bincode::serialize(&tx)?;
    let b58_tx = bs58::encode(serialized).into_string();

    // 发送到Jito
    let response: Value = http.post("https://mainnet.block-engine.jito.wtf/api/v1/transactions")
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "sendTransaction",
            "params": [b58_tx]
        }))
        .send().await?
        .json().await?;

    let sig_str = response["result"].as_str().ok_or("No signature in response")?;
    let signature = Signature::from_str(sig_str)?;
    
    println!("Transaction sent: {}", signature);
    Ok(signature)
}