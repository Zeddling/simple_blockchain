extern crate getrandom;
extern crate hex;
extern crate oorandom;
extern crate pretty_env_logger;
extern crate serde;
extern crate sha2;

use byteorder::{BigEndian, ReadBytesExt};
use chrono::offset::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Cursor;

const DIFFICULTY_PREFIX: &str = "00";

//  Represents the entire chain in the network
pub struct Chain {
    //  Actual chain
    pub blocks: Vec<Block>,
}

//  Represents a single block in the blockchain
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Block {
    pub id: u64,

    //  Hash representing block
    pub hash: String,

    //  Hash of the previous block
    pub previous_hash: String,

    //  Time of creation in UTC
    pub timestamp: i64,

    //  Data contained in the block
    pub data: String,

    //  Value for hashing the block(PoW)
    pub nonce: u64,
}

impl Block {
    pub fn new(id: u64, previous_hash: String, data: String) -> Self {
        let now = Utc::now();
        let (nonce, hash) = mine_block(id, &data, &previous_hash, now.timestamp());
        Self {
            id,
            hash,
            timestamp: now.timestamp(),
            previous_hash,
            data,
            nonce,
        }
    }
}

/**
 * Implementation of basic blockchain functions
 *
 * Consesus algorithm: Searches for the longest chain
 */
impl Chain {
    //  Chooses the longest valid chain
    pub fn choose(&mut self, local: Vec<Block>, remote: Vec<Block>) -> Vec<Block> {
        let is_local_valid = self.is_chain_valid(&local);
        let is_remote_valid = self.is_chain_valid(&remote);

        if is_local_valid && is_remote_valid {
            if local.len() >= remote.len() {
                local
            } else {
                remote
            }
        } else if is_remote_valid && !is_local_valid {
            remote
        } else if !is_remote_valid && is_local_valid {
            local
        } else {
            panic!("Local and remote chains are invalid");
        }
    }

    //  Creates the genesis block for our chain
    pub fn genesis(&mut self) {
        let genesis_block = Block {
            id: 0,
            timestamp: Utc::now().timestamp(),
            previous_hash: String::from("genesis"),
            data: String::from("genesis!"),
            nonce: 2836,
            hash: "0000f816a87f806bb0073dcf026a64fb40c946b5abee2573702828694d5b4c43".to_string(),
        };

        self.blocks.push(genesis_block);
    }

    //  Asserts the validity of a block
    fn is_block_valid(&self, block: &Block, previous_block: &Block) -> bool {
        if block.previous_hash != previous_block.hash {
            warn!("block with id: {} has wrong previous hash", block.id);
            return false;
        } else if !hash_to_binary_representation(
            &hex::decode(&block.hash).expect("can decode from hex"),
        )
        .starts_with(DIFFICULTY_PREFIX)
        {
            warn!("block with id: {} has an invalid difficulty", block.id);
            return false;
        } else if block.id != previous_block.id + 1 {
            warn!(
                "block with id: {} is not the next block after the latest: {}",
                block.id, previous_block.id
            );
            return false;
        } else if hex::encode(calculate_hash(
            block.data.as_ref(),
            block.id,
            block.nonce,
            block.previous_hash.as_ref(),
            block.timestamp,
        )) != block.hash
        {
            warn!("block with id: {} has invalid hash", block.id);
            return false;
        }
        true
    }

    //  Checks whether the chain is valid
    fn is_chain_valid(&self, chain: &[Block]) -> bool {
        for i in 0..chain.len() {
            if i == 0 {
                continue;
            }

            let first = chain.get(i - 1).expect("has to exist");
            let second = chain.get(i).expect("has to exist");
            if !self.is_block_valid(second, first) {
                return false;
            }
        }

        true
    }

    //  Creates a new chain
    pub fn new() -> Self {
        Self { blocks: vec![] }
    }

    //  Attempts too add block to chain
    pub fn try_add_block(&mut self, block: Block) {
        let latest_block = self.blocks.last().expect("there is at least one block");
        if self.is_block_valid(&block, &latest_block) {
            self.blocks.push(block);
        } else {
            error!("could not add block - invalid");
        }
    }
}

//  Calculates hash
fn calculate_hash(data: &str, id: u64, nonce: u64, previous_hash: &str, timestamp: i64) -> Vec<u8> {
    let data = serde_json::json!({
        "data": data,
        "id": id,
        "nonce": nonce,
        "previous_hash": previous_hash,
        "timestamp": timestamp
    });

    let mut hasher = Sha256::new();
    hasher.update(data.to_string().as_bytes());
    hasher.finalize().as_slice().to_owned()
}

//  Generate random number
fn generate_random() -> u64 {
    //  Generate secure random
    //  Get seed
    let mut buf = [0u8; 32];
    getrandom::getrandom(&mut buf);

    let mut reader = Cursor::new(buf);
    let seed: u64 = reader.read_u64::<BigEndian>().unwrap();

    //  Get random
    let mut rng = oorandom::Rand32::new(seed);
    rng.rand_u32() as u64
}

//  Binary representation of the hash array as a string
fn hash_to_binary_representation(hash: &[u8]) -> String {
    let mut result: String = String::default();

    for element in hash {
        result.push_str(&format!("{:b}", element));
    }

    result
}

//  Searches for a random nonce where first the hash satisfies our
//  difficulty prefix
fn mine_block(id: u64, data: &str, previous_hash: &str, timestamp: i64) -> (u64, String) {
    info!("Mining block {}", id);

    let mut nonce = generate_random();

    loop {
        let hash = calculate_hash(data.as_ref(), id, nonce, previous_hash.as_ref(), timestamp);
        let binary_hash = hash_to_binary_representation(&hash);

        if binary_hash.starts_with(DIFFICULTY_PREFIX) {
            info!(
                "mined! nonce: {}, hash: {}, binary hash: {}",
                nonce,
                hex::encode(&hash),
                binary_hash
            );
            return (nonce, hex::encode(hash));
        }

        nonce = generate_random();
    }
}
