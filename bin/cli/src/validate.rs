// Copyright 2024 RISC Zero, Inc.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use crate::channel::DuplexChannel;
use crate::propose::Proposal;
use crate::{output_at_block, FAULT_PROOF_GAME_TYPE};
use alloy::network::EthereumWallet;
use alloy::primitives::{Address, FixedBytes, U256};
use alloy::providers::{Provider, ProviderBuilder};
use alloy::signers::local::LocalSigner;
use anyhow::{bail, Context};
use kailua_client::fpvm_proof_file_name;
use kailua_contracts::IDisputeGameFactory::gameAtIndexReturn;
use kailua_contracts::{FaultProofGame, IAnchorStateRegistry, IDisputeGameFactory};
use kailua_host::fetch_rollup_config;
use risc0_zkvm::Receipt;
use std::collections::HashMap;
use std::env;
use std::process::exit;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use tokio::fs::File;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::time::sleep;
use tokio::{spawn, try_join};
use tracing::{debug, error, info, warn};

#[derive(clap::Args, Debug, Clone)]
pub struct ValidateArgs {
    #[arg(long, short, help = "Verbosity level (0-4)", action = clap::ArgAction::Count)]
    pub v: u8,

    /// Address of OP-NODE endpoint to use
    #[clap(long)]
    pub op_node_address: String,
    /// Address of L2 JSON-RPC endpoint to use (eth and debug namespace required).
    #[clap(long)]
    pub l2_node_address: String,
    /// Address of L1 JSON-RPC endpoint to use (eth namespace required)
    #[clap(long)]
    pub l1_node_address: String,
    /// Address of the L1 Beacon API endpoint to use.
    #[clap(long)]
    pub l1_beacon_address: String,

    /// Address of the L1 `AnchorStateRegistry` contract
    #[clap(long)]
    pub registry_contract: String,

    /// Secret key of L1 wallet to use for challenging and proving outputs
    #[clap(long)]
    pub validator_key: String,
}

pub async fn validate(args: ValidateArgs) -> anyhow::Result<()> {
    // We run two concurrent tasks, one for the chain, and one for the prover.
    // Both tasks communicate using the duplex channel
    let channel_pair = DuplexChannel::new_pair(4096);

    let handle_proposals = spawn(handle_proposals(channel_pair.0, args.clone()));
    let handle_proofs = spawn(handle_proofs(channel_pair.1, args));

    let (proposals_task, proofs_task) = try_join!(handle_proposals, handle_proofs)?;
    proposals_task.context("handle_proposals")?;
    proofs_task.context("handle_proofs")?;

    Ok(())
}

#[derive(Clone, Debug)]
pub enum Message {
    // The proposal and its parent
    Proposal {
        local_index: usize,
        l1_head: FixedBytes<32>,
        l2_head: FixedBytes<32>,
        l2_output_root: FixedBytes<32>,
        l2_block_number: u64,
        l2_claim: FixedBytes<32>,
    },
    Proof(usize, Receipt),
}

pub async fn handle_proofs(
    mut channel: DuplexChannel<Message>,
    args: ValidateArgs,
) -> anyhow::Result<()> {
    // Fetch rollup configuration
    let l2_chain_id = fetch_rollup_config(&args.op_node_address, &args.l2_node_address, None)
        .await?
        .l2_chain_id
        .to_string();
    // Read executable paths from env vars
    let kailua_host = env::var("KAILUA_HOST").unwrap_or_else(|_| {
        warn!("KAILUA_HOST set to default ./target/debug/kailua-host");
        String::from("./target/debug/kailua-host")
    });
    let kailua_client = env::var("KAILUA_CLIENT").unwrap_or_else(|_| {
        warn!("KAILUA_CLIENT set to default ./target/debug/kailua-client");
        String::from("./target/debug/kailua-client")
    });
    let data_dir = env::var("KAILUA_DATA").unwrap_or_else(|_| {
        warn!("KAILUA_DATA set to default .localtestdata");
        String::from(".localtestdata")
    });
    // Run proof generator loop
    loop {
        // Dequeue messages
        // todo: priority goes to fault proofs for games where one is the challenger
        // todo: secondary priority is validity proofs for mis-challenged games
        let Message::Proposal {
            local_index,
            l1_head,
            l2_head,
            l2_output_root,
            l2_block_number,
            l2_claim,
        } = channel
            .receiver
            .recv()
            .await
            .expect("proof receiver channel closed")
        else {
            bail!("Unexpected message type.");
        };
        info!("Processing proof for local index {local_index}.");
        // Prepare kailua-host parameters
        let proof_file_name = fpvm_proof_file_name(l1_head, l2_claim);
        let l1_head = l1_head.to_string();
        let l2_head = l2_head.to_string();
        let l2_output_root = l2_output_root.to_string();
        let l2_claim = l2_claim.to_string();
        let l2_block_number = l2_block_number.to_string();
        let verbosity = [
            String::from("-"),
            (0..args.v).map(|_| 'v').collect::<String>(),
        ]
        .concat();
        let mut proving_args = vec![
            "--l1-head", // l1 head from on-chain proposal
            &l1_head,
            "--l2-head", // l2 starting block hash from on-chain proposal
            &l2_head,
            "--l2-output-root", // l2 starting output root
            &l2_output_root,
            "--l2-claim", // proposed output root
            &l2_claim,
            "--l2-block-number", // proposed block number
            &l2_block_number,
            "--l2-chain-id", // rollup chain id
            &l2_chain_id,
            "--l1-node-address", // l1 el node
            &args.l1_node_address,
            "--l1-beacon-address", // l1 cl node
            &args.l1_beacon_address,
            "--l2-node-address", // l2 el node
            &args.l2_node_address,
            "--op-node-address", // l2 cl node
            &args.op_node_address,
            "--exec", // path to kailua-client
            &kailua_client,
            "--data-dir", // path to cache
            &data_dir,
        ];
        // verbosity level
        if args.v > 0 {
            proving_args.push(&verbosity);
        }
        debug!("proving_args {:?}", &proving_args);
        // Prove via kailua-host (re dev mode/bonsai: env vars inherited!)
        let proving_task = Command::new(&kailua_host)
            .args(proving_args)
            .spawn()
            .context("Invoking kailua-host")?
            .wait()
            .await?;
        if !proving_task.success() {
            error!("Proving task failure.");
        }
        // Read receipt file
        let mut receipt_file = File::open(proof_file_name.clone()).await?;
        let mut receipt_data = Vec::new();
        receipt_file.read_to_end(&mut receipt_data).await?;
        let receipt: Receipt = bincode::deserialize(&receipt_data)?;
        // Send proof via the channel
        channel
            .sender
            .send(Message::Proof(local_index, receipt))
            .await?;
        info!("Proof for local index {local_index} complete.");
    }
}

pub async fn handle_proposals(
    mut channel: DuplexChannel<Message>,
    args: ValidateArgs,
) -> anyhow::Result<()> {
    // connect to l2 cl node
    let op_node_provider =
        ProviderBuilder::new().on_http(args.op_node_address.as_str().try_into()?);
    let l2_node_provider =
        ProviderBuilder::new().on_http(args.l2_node_address.as_str().try_into()?);
    // initialize validator wallet
    info!("Initializing validator wallet.");
    let validator_signer = LocalSigner::from_str(&args.validator_key)?;
    let validator_address = validator_signer.address();
    let validator_wallet = EthereumWallet::from(validator_signer);
    let validator_provider = Arc::new(
        ProviderBuilder::new()
            .with_recommended_fillers()
            .wallet(validator_wallet)
            .on_http(args.l1_node_address.as_str().try_into()?),
    );
    // Init registry and factory contracts
    let anchor_state_registry = IAnchorStateRegistry::new(
        Address::from_str(&args.registry_contract)?,
        validator_provider.clone(),
    );
    info!("AnchorStateRegistry({:?})", anchor_state_registry.address());
    let dispute_game_factory = IDisputeGameFactory::new(
        anchor_state_registry.disputeGameFactory().call().await?._0,
        validator_provider.clone(),
    );
    info!("DisputeGameFactory({:?})", dispute_game_factory.address());
    let game_count: u64 = dispute_game_factory
        .gameCount()
        .call()
        .await?
        .gameCount_
        .to();
    info!("There have been {game_count} games created using DisputeGameFactory");
    let fault_proof_game_implementation = FaultProofGame::new(
        dispute_game_factory
            .gameImpls(FAULT_PROOF_GAME_TYPE)
            .call()
            .await?
            .impl_,
        validator_provider.clone(),
    );
    info!(
        "FaultProofGame({:?})",
        fault_proof_game_implementation.address()
    );
    if fault_proof_game_implementation.address().is_zero() {
        error!("Fault proof game is not installed!");
        exit(1);
    }
    // load constants
    let _max_proposal_span: u64 = fault_proof_game_implementation
        .maxBlockCount()
        .call()
        .await?
        .maxBlockCount_
        .to();
    let bond_value = dispute_game_factory
        .initBonds(FAULT_PROOF_GAME_TYPE)
        .call()
        .await?
        .bond_;
    // Initialize empty state
    info!("Initializing..");
    let mut proposal_tree: Vec<Proposal> = vec![];
    let mut proposal_index = HashMap::new();
    let mut search_start_index = 0;
    // Run the validator loop
    loop {
        // validate latest games
        let game_count: u64 = dispute_game_factory
            .gameCount()
            .call()
            .await?
            .gameCount_
            .to();
        for factory_index in search_start_index..game_count {
            let gameAtIndexReturn {
                gameType_: game_type,
                proxy_: game_address,
                ..
            } = dispute_game_factory
                .gameAtIndex(U256::from(factory_index))
                .call()
                .await
                .context(format!("gameAtIndex {factory_index}/{game_count}"))?;
            // skip entries for other game types
            if game_type != FAULT_PROOF_GAME_TYPE {
                continue;
            }
            info!("Processing proposal at factory index {factory_index}");
            // Retrieve basic data
            let game_contract = FaultProofGame::new(game_address, dispute_game_factory.provider());
            let output_root = game_contract.rootClaim().call().await?.rootClaim_;
            let output_block_number = game_contract.l2BlockNumber().call().await?.l2BlockNumber_;
            let resolved = game_contract.resolvedAt().call().await?._0 > 0;
            let extra_data = game_contract.extraData().call().await?.extraData_;
            let local_index = proposal_tree.len();
            // Retrieve game/setup data
            let (parent_local_index, mut challenged, proven) = match extra_data.len() {
                0x10 => {
                    // FaultProofGame instance
                    let parent_factory_index = game_contract
                        .parentGameIndex()
                        .call()
                        .await?
                        .parentGameIndex_;
                    let Some(parent_local_index) = proposal_index.get(&parent_factory_index) else {
                        error!("SKIPPED: Could not find parent local index for game {game_address} at factory index {factory_index}.");
                        continue;
                    };
                    let challenged = game_contract.challengedAt().call().await?._0 > 0;
                    let proven = game_contract.provenAt().call().await?._0 > 0;
                    (*parent_local_index, challenged, proven)
                }
                0x20 => {
                    // FaultProofSetup instance
                    (local_index, false, false)
                }
                _ => bail!("Unexpected extra data length from game {game_address} at factory index {factory_index}")
            };
            // Decide correctness according to op-node
            let local_output_root =
                output_at_block(&op_node_provider, output_block_number.to()).await?;
            let correct = if local_output_root != output_root {
                // op-node disagrees, so this must be invalid
                warn!("Encountered an incorrect proposal {output_root} for block {output_block_number}! Expected {local_output_root}.");
                false
            } else if parent_local_index != local_index {
                // FaultProofGame can only be valid if parent is valid
                proposal_tree[parent_local_index].correct
            } else {
                // FaultProofSetup is self evident if op-node agrees
                true
            };
            // challenge any unchallenged bad proposals
            if !correct && !challenged {
                // Issue challenge against incorrect unchallenged proposals
                info!("Challenging bad proposal.");
                game_contract
                    .challenge()
                    .value(bond_value / U256::from(2))
                    .send()
                    .await
                    .context("challenge (send)")?
                    .get_receipt()
                    .await
                    .context("challenge (get_receipt)")?;
                challenged = true;
            }
            // update local tree view
            proposal_index.insert(factory_index, local_index);
            info!("Validated {correct} proposal at factory index {factory_index}");
            let output_block_number = output_block_number.to();
            proposal_tree.push(Proposal {
                factory_index,
                game_address,
                parent_local_index,
                output_root,
                output_block_number,
                challenged,
                proven,
                resolved,
                correct,
            });
            // enqueue proving for any bad proposals challenged by this validator
            if challenged
                && !proven
                && game_contract.challenger().call().await?._0 == validator_address
            {
                // Read additional data for Kona invocation
                info!("Requesting proof for local index {local_index}.");
                let l1_head = game_contract
                    .l1Head()
                    .call()
                    .await
                    .context("l1Head")?
                    .l1Head_;
                debug!("l1_head {:?}", &l1_head);
                let l2_head_number: u64 = game_contract
                    .startingBlockNumber()
                    .call()
                    .await
                    .context("startingBlockNumber")?
                    .startingBlockNumber_
                    .to();
                debug!("l2_head_number {:?}", &l2_head_number);
                let l2_head_block: serde_json::Value = l2_node_provider
                    .client()
                    .request(
                        "eth_getBlockByNumber",
                        (format!("0x{:x}", l2_head_number), false),
                    )
                    .await
                    .context(format!("eth_getBlockByNumber {l2_head_number}"))?;
                debug!("l2_head_block {:?}", &l2_head_block);
                let l2_head = FixedBytes::<32>::from_str(
                    l2_head_block["hash"]
                        .as_str()
                        .expect("Failed to parse block hash"),
                )?;
                debug!("l2_head {:?}", &l2_head);
                let l2_output_root = game_contract
                    .startingRootHash()
                    .call()
                    .await?
                    .startingRootHash_;
                let local_output_root = output_at_block(&op_node_provider, l2_head_number).await?;
                // We can only resolve this challenged game once the bad parent is resolved, so we skip proving.
                if l2_output_root != local_output_root {
                    warn!("Skipping proving for challenged local index {local_index} with bad parent output.");
                    let parent = &proposal_tree[parent_local_index];
                    if parent.challenged {
                        info!(
                            "{} parent of local index {local_index} is already challenged.",
                            parent.correct
                        );
                    } else {
                        error!(
                            "{} parent of local index {local_index} is NOT challenged!",
                            parent.correct
                        );
                    }
                    if parent.correct {
                        error!("Parent {parent_local_index} of {local_index} is correct!");
                    }
                    continue;
                }
                // Message proving task
                channel
                    .sender
                    .send(Message::Proposal {
                        local_index,
                        l1_head,
                        l2_head,
                        l2_output_root,
                        l2_block_number: output_block_number,
                        l2_claim: output_root,
                    })
                    .await?;
            }
        }
        search_start_index = game_count;
        // publish computed proofs
        while !channel.receiver.is_empty() {
            let Message::Proof(local_index, receipt) = channel
                .receiver
                .recv()
                .await
                .expect("proposals receiver channel closed")
            else {
                bail!("Unexpected message type.");
            };
            let proposal = &proposal_tree[local_index];
            let game_contract =
                FaultProofGame::new(proposal.game_address, dispute_game_factory.provider());
            let is_fault_proof = *receipt.journal.bytes.last().unwrap() > 0;
            let proof_label = if is_fault_proof { "fault" } else { "validity" };
            info!(
                "Utilizing {proof_label} proof in game at {}",
                proposal.game_address
            );
            // only prove unproven games
            if game_contract.proofStatus().call().await?._0 == 0 {
                let snark = receipt.inner.groth16()?;
                game_contract
                    .prove(snark.seal.clone().into(), is_fault_proof)
                    .send()
                    .await?
                    .get_receipt()
                    .await?;
                info!("Proof submitted!");
            } else {
                warn!("Skipping proof submission for already proven game at local index {local_index}.");
            }
        }

        // Wait for new data
        sleep(Duration::from_secs(1)).await;
    }
}
