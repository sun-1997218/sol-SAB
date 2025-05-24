use std::{env, time::{SystemTime, UNIX_EPOCH},str::FromStr};
use base64::Engine;
use bs58;
use reqwest::Client;
use serde_json::{json, Value};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::{
    
    commitment_config::CommitmentConfig,
    compute_budget::ComputeBudgetInstruction,
    instruction::{AccountMeta, Instruction},
    message::{v0, VersionedMessage},
    pubkey::Pubkey,
    signature::{Keypair, Signer},
    system_instruction::transfer,
    transaction:: VersionedTransaction,
};
use tokio::time::{sleep, Duration};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenv::dotenv().ok();
    let secret_key = env::var("SECRET_KEY")?;
    let payer = Keypair::from_base58_string(&secret_key);
    println!("payer: {}", payer.pubkey());

    let connection = RpcClient::new_with_commitment(
        "https://mainnet.helius-rpc.com/?api-key=77a3d694-a103-48d1-9c07-fa80651d5390".to_string(),
        CommitmentConfig::processed(),
    );

    let quote_url = "https://api.jup.ag/swap/v1/quote";
    let swap_instruction_url = "https://lite-api.jup.ag/swap/v1/swap-instructions";

    let w_sol_mint = "So11111111111111111111111111111111111111112";
    let usdc_mint = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";

    loop {
        run(&connection, &payer, quote_url, swap_instruction_url, w_sol_mint, usdc_mint).await?;
        sleep(Duration::from_millis(200)).await;
    }
}

async fn get_program_id(connection: &RpcClient, mint_address: &str) -> Result<String, Box<dyn std::error::Error>> {
    let pubkey = Pubkey::from_str(mint_address)?;
    let account = connection.get_account(&pubkey).await?;
    Ok(account.owner.to_string())
}

fn instruction_format(instruction: &Value) -> Instruction {
    let program_id = Pubkey::from_str(instruction["programId"].as_str().unwrap()).unwrap();
    let accounts = instruction["accounts"].as_array().unwrap().iter().map(|acc| {
        AccountMeta {
            pubkey: Pubkey::from_str(acc["pubkey"].as_str().unwrap()).unwrap(),
            is_signer: acc["isSigner"].as_bool().unwrap(),
            is_writable: acc["isWritable"].as_bool().unwrap(),
        }
    }).collect();
    
    let data = base64::engine::general_purpose::STANDARD.decode(instruction["data"].as_str().unwrap()).unwrap();
    
    Instruction {
        program_id,
        accounts,
        data,
    }
}

async fn fetch_alt_accounts(connection: &RpcClient, keys: &[String]) -> Result<Vec<solana_sdk::address_lookup_table::AddressLookupTableAccount>, Box<dyn std::error::Error>> {
    let mut alt_accounts = Vec::new();
    for key in keys {
        let account = connection.get_account(&Pubkey::from_str(key)?).await?;
        let addresses: Vec<Pubkey> = serde_json::from_slice::<Value>(&account.data)?
            .get("info")
            .and_then(|v| v.get("addresses"))
            .and_then(|v| v.as_array())
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str())
            .map(|s| Pubkey::from_str(s).unwrap())
            .collect();
        alt_accounts.push(solana_sdk::address_lookup_table::AddressLookupTableAccount {
            key: Pubkey::from_str(key)?,
            addresses,
        });
    }
    Ok(alt_accounts)
}

async fn run(
    connection: &RpcClient,
    payer: &Keypair,
    quote_url: &str,
    swap_instruction_url: &str,
    w_sol_mint: &str,
    usdc_mint: &str
) -> Result<(), Box<dyn std::error::Error>> {
    let start = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();

    // Quote0: WSOL -> USDC
    let quote0_params = json!({
        "inputMint": w_sol_mint,
        "outputMint": usdc_mint,
        "amount": 1000000,
        "onlyDirectRoutes": false,
        "slippageBps": 100,
        "maxAccounts": 20
    });

    let quote0_resp: Value = Client::new()
        .get(quote_url)
        .query(&quote0_params)
        .send()
        .await?
        .json()
        .await?;

    // Quote1: USDC -> WSOL
    let quote1_params = json!({
        "inputMint": usdc_mint,
        "outputMint": w_sol_mint,
        "amount": quote0_resp["outAmount"],
        "onlyDirectRoutes": false,
        "slippageBps": 20,
        "maxAccounts": 20
    });

    let quote1_resp: Value = Client::new()
        .get(quote_url)
        .query(&quote1_params)
        .send()
        .await?
        .json()
        .await?;

    let diff_lamports = quote1_resp["outAmount"].as_u64().unwrap() - 1000000;
    println!("diff_lamports: {}", diff_lamports);

    if diff_lamports > 1000 {
        let mut merged_quote = quote0_resp.clone();
        merged_quote["outputMint"] = json!(usdc_mint);
        merged_quote["outAmount"] = json!(quote0_resp["outAmount"].as_u64().unwrap() + 1000);
        merged_quote["priceImpactPct"] = json!("0");

        let swap_data = json!({
            "userPublicKey": payer.pubkey().to_string(),
            "quoteResponse": merged_quote,
            "wrapAndUnwrapSol": true,
            "onlyDirectRoutes": true,
            "useSharedAccounts": false,
            "computeUnitPriceMicroLamports": 5000,
            "dynamicComputeUnitLimit": true,
            "skipUserAccountsRpcCalls": true,
            "dynamicSlippage": true,
            "skipInitializeImmutableOwner": true
        });

        let instructions_resp: Value = Client::new()
            .post(swap_instruction_url)
            .json(&swap_data)
            .send()
            .await?
            .json()
            .await?;

        let mut ixs = vec![
            ComputeBudgetInstruction::set_compute_unit_limit(instructions_resp["computeUnitLimit"].as_u64().unwrap() as u32)
        ];

        // Setup instructions
        for instr in instructions_resp["setupInstructions"].as_array().unwrap() {
            ixs.push(instruction_format(instr));
        }

        // Swap instruction
        ixs.push(instruction_format(&instructions_resp["swapInstruction"]));

        // Cleanup instruction
        if let Some(cleanup) = instructions_resp.get("cleanupInstruction") {
            ixs.push(instruction_format(cleanup));
        }

        // Jito tip
        let tip_ix = transfer(
            &payer.pubkey(),
            &Pubkey::from_str("96gYZGLnJYVFmbjzopPSU6QiEV5fGqZNyN9nmNhvrZU5")?,
            1000
        );
        ixs.push(tip_ix);

        // Build transaction
        let blockhash = connection.get_latest_blockhash().await?;
        let message = VersionedMessage::V0(v0::Message::try_compile(
            &payer.pubkey(),
            &ixs,
            &[],
            blockhash
        )?);

        let transaction = VersionedTransaction::try_new(
            message,
            &[payer]
        )?;

        // Serialize and send
        let serialized = bincode::serialize(&transaction)?;
        let base58_tx = bs58::encode(serialized).into_string();

        let bundle = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "sendTransaction",
            "params": [base58_tx]
        });

        let response: Value = Client::new()
            .post("https://mainnet.block-engine.jito.wtf/api/v1/transactions")
            .json(&bundle)
            .send()
            .await?
            .json()
            .await?;

        println!("Bundle ID: {}", response["result"]);
    }

    Ok(())
}