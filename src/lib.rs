use std::collections::HashMap;
use near_sdk::borsh::{self, BorshSerialize, BorshDeserialize};
use near_sdk::collections::{LazyOption, UnorderedSet, UnorderedMap};
use near_sdk::json_types::{ValidAccountId, U128};
use near_sdk::serde::{Deserialize, Serialize};
use near_sdk::{
  env, near_bindgen, ext_contract, AccountId, PanicOnDefault, 
  BorshStorageKey, Promise
};

use near_contract_standards::non_fungible_token::{Token, TokenId, NonFungibleToken};
use near_contract_standards::non_fungible_token::metadata::{
  NFTContractMetadata, NonFungibleTokenMetadataProvider, TokenMetadata, NFT_METADATA_SPEC,
};

mod metadatas;

near_sdk::setup_alloc!();

const MINIMUM_EARLY_DEPOSIT: u128 = 10u128.pow(24);
pub type MetadataType = String;

#[derive(Serialize, Deserialize)]
#[serde(crate = "near_sdk::serde")]
pub struct JsonToken {
  pub token_id: TokenId,
  pub owner_id: AccountId,
  pub metadata: TokenMetadata,
}

#[near_bindgen]
#[derive(BorshSerialize, BorshDeserialize, PanicOnDefault)]
pub struct Contract {
  owner_id: AccountId,
  tokens: NonFungibleToken,
  metadata_per_type: UnorderedMap<MetadataType, UnorderedSet<TokenMetadata>>,
  egg_per_token_id: UnorderedMap<TokenId, u64>,
  level_per_token_id: UnorderedMap<TokenId, u64>,
  metadata: LazyOption<NFTContractMetadata>,
  current_token_id: TokenId,
}

#[derive(BorshSerialize, BorshStorageKey)]
enum StorageKey {
  NonFungibleToken,
  TokenMetadata,
  Enumeration,
  Approval,
  TokensPerOwner { account_hash: Vec<u8> },
  MetadataPerType,
  Metadata,
  MetadataPerTypeInner,
  EggPerTokenId,
  LevelPerTokenId,
}

pub trait NonFungibleTokenCore {
  fn nft_approve(&mut self, token_id: TokenId, account_id: AccountId, msg: Option<String>);

  fn nft_is_approved(&self, token_id: TokenId, approved_account_id: AccountId, approval_id: Option<u64>);

  fn nft_revoke(&mut self, token_id: TokenId, account_id: AccountId);

  fn nft_revoke_all(&mut self, token_id: TokenId);
}

#[ext_contract(ext_non_fungible_approval_receiver)]
trait NonFungibleTokenApprovalsReceiver {
  fn nft_on_approve(&mut self, token_id: TokenId, owner_id: AccountId, approval_id: u64, msg: String);
}

#[near_bindgen]
impl Contract {
  #[init]
  pub fn new_default_meta(owner_id: ValidAccountId) -> Self {
    Self::new(
      owner_id,
      NFTContractMetadata {
        spec: NFT_METADATA_SPEC.to_string(),
        name: "Nearmon".to_string(),
        symbol: "NMON".to_string(),
        icon: None,
        base_uri: None,
        reference: None,
        reference_hash: None,
      },
    )
  }

  #[init]
  pub fn new(
    owner_id: ValidAccountId,
    metadata: NFTContractMetadata,
  ) -> Self {
    assert!(!env::state_exists(), "Already Initialized");
    metadata.assert_valid();
    let owner = owner_id.to_string();
    Self {
      owner_id: owner,
      tokens: NonFungibleToken::new(
        StorageKey::NonFungibleToken,
        owner_id,
        Some(StorageKey::TokenMetadata),
        Some(StorageKey::Enumeration),
        Some(StorageKey::Approval),
      ),
      egg_per_token_id: UnorderedMap::new(StorageKey::EggPerTokenId),
      level_per_token_id: UnorderedMap::new(StorageKey::LevelPerTokenId),
      metadata_per_type: UnorderedMap::new(StorageKey::MetadataPerType),
      metadata: LazyOption::new(
        StorageKey::Metadata.try_to_vec().unwrap(),
        Some(&metadata),
      ),
      current_token_id: String::from("0"),
    }
  }

  #[payable]
  pub fn add_metadata(
    &mut self,
    metadata_type: MetadataType,
    metadata: TokenMetadata,
  ) {
    let caller_id = env::signer_account_id();
    let lower_type = metadata_type.to_lowercase();

    assert_eq!(
      caller_id,
      self.owner_id,
      "Unauthorized",
    );

    let mut metadata_set = self.metadata_per_type.get(&lower_type).unwrap_or_else(|| {
      UnorderedSet::new(StorageKey::MetadataPerTypeInner)
    });

    metadata_set.insert(&metadata);

    self.metadata_per_type.insert(&lower_type, &metadata_set);
  }

  #[payable]
  pub fn nft_mint_egg(
    &mut self,
    receiver_id: AccountId,
  ) {
    self.increment_token_id();

    let metadata_type = 0u64;
    let owner_id: AccountId = receiver_id;
    let mut metadata_set = self.get_random_number() as u64;
    metadata_set = metadata_set % 16 + 1;

    let metadata: TokenMetadata = self.get_metadata_per_type(metadata_type, metadata_set);

    self.tokens.owner_by_id.insert(&self.current_token_id, &owner_id);

    self.egg_per_token_id.insert(&self.current_token_id, &metadata_set);

    self.level_per_token_id.insert(&self.current_token_id, &0u64);
    
    self.tokens
      .token_metadata_by_id
      .as_mut()
      .and_then(|by_id| by_id.insert(&self.current_token_id, &metadata));

    if let Some(tokens_per_owner) = &mut self.tokens.tokens_per_owner {
      let mut token_ids = tokens_per_owner.get(&owner_id).unwrap_or_else(|| {
        UnorderedSet::new(StorageKey::TokensPerOwner {
          account_hash: env::sha256(&owner_id.as_bytes()),
        })
      });
      token_ids.insert(&self.current_token_id);
      tokens_per_owner.insert(&owner_id, &token_ids);
    }

    refund_deposit();
  }

  #[payable]
  pub fn nft_evolve(
    &mut self,
    token_id: TokenId,
    receiver_id: AccountId,
  ) {    
    self.increment_token_id();
    let evolve_time;

    if let Some(temp_metadata) = &self.tokens.token_metadata_by_id {
      evolve_time = temp_metadata.get(&token_id).unwrap().extra.unwrap();
      
      if env::block_timestamp() / 1000000 < evolve_time.parse::<u64>().unwrap() {
        panic!("The evolve time is not fullfiled");
      }
    }

    let owner_id = self.tokens.owner_by_id.get(&token_id).unwrap();
    assert_eq!(
      owner_id,
      env::predecessor_account_id(),
      "You are not the Token owner",
    );

    if let Some(next_approval_id_by_id) = &mut self.tokens.next_approval_id_by_id {
      next_approval_id_by_id.remove(&token_id);
    }

    if let Some(approvals_by_id) = &mut self.tokens.approvals_by_id {
      approvals_by_id.remove(&token_id);
    }

    if let Some(tokens_per_owner) = &mut self.tokens.tokens_per_owner {
      let mut token_set = tokens_per_owner.get(&receiver_id).unwrap();
      token_set.remove(&token_id);
      tokens_per_owner.insert(&receiver_id, &token_set);
    } 

    if let Some(token_metadata_by_id) = &mut self.tokens.token_metadata_by_id {
      token_metadata_by_id.remove(&token_id);
    }

    let metadata_set = self.egg_per_token_id.get(&token_id).unwrap();
    self.egg_per_token_id.insert(&self.current_token_id, &metadata_set);

    let mut token_level = self.level_per_token_id.get(&token_id).unwrap();
    token_level = token_level + 1;
    self.level_per_token_id.remove(&token_id);
    self.level_per_token_id.insert(&self.current_token_id, &token_level);    

    let metadata: TokenMetadata = self.get_metadata_per_type(token_level, metadata_set);
    self.tokens.owner_by_id.insert(&self.current_token_id, &owner_id);

    self.tokens
      .token_metadata_by_id
      .as_mut()
      .and_then(|by_id| by_id.insert(&self.current_token_id, &metadata));

    if let Some(tokens_per_owner) = &mut self.tokens.tokens_per_owner {
      let mut token_ids = tokens_per_owner.get(&owner_id).unwrap_or_else(|| {
        UnorderedSet::new(StorageKey::TokensPerOwner {
          account_hash: env::sha256(&owner_id.as_bytes()),
        })
      });
      token_ids.insert(&self.current_token_id);
      tokens_per_owner.insert(&owner_id, &token_ids);
    }

    refund_deposit()
  }

  pub fn nft_tokens_for_owner(
    &self,
    account_id: AccountId,
    from_index: Option<U128>,
    limit: Option<u64>
  ) -> Vec<Token> {
    let tokens_per_owner = self.tokens.tokens_per_owner.as_ref().expect(
      "Could not find tokens_per_owner when calling a method on the enumeration standards",
    );

    let token_set = if let Some(token_set) = tokens_per_owner.get(&account_id) {
      token_set
    } else {
      return vec![];
    };

    let keys = token_set.as_vector();

    let start = u128::from(from_index.unwrap_or(U128(0)));

    keys
      .iter()
      .skip(start as usize)
      .take(limit.unwrap_or(0) as usize)
      .map(|token| self.nft_token(token).unwrap())
      .collect()
  }

  pub fn nft_token(
    &self, 
    token_id: TokenId,
  ) -> Option<Token> {
    let owner_id = self.tokens.owner_by_id.get(&token_id)?;
    let approved_account_ids = self
      .tokens
      .approvals_by_id
      .as_ref()
      .and_then(|by_id| by_id.get(&token_id).or_else(|| Some(HashMap::new())));

    let token_metadata = self.tokens.token_metadata_by_id.as_ref().unwrap().get(&token_id).unwrap();

    Some(Token {
      token_id,
      owner_id,
      metadata: Some(token_metadata),
      approved_account_ids,
    })
  }

  pub fn get_owner(&self) -> AccountId {
    self.tokens.owner_id.clone()
  }

  pub fn metadata_type_list(&self) -> Vec<String> {
    let metadata_vector = self.metadata_per_type.keys_as_vector();
    metadata_vector.to_vec()
  }

  pub fn metadata_per_type_list(
    &self,
    metadata_type: MetadataType,
  ) -> Vec<TokenMetadata> {
    let metadata_vector = if let Some(metadata_set) = self.metadata_per_type.get(&metadata_type) {
      metadata_set
    } else {
      return vec![];
    };

    metadata_vector.to_vec()
  }

  pub fn level_per_token(
    &self, 
    token_id: TokenId,
  ) -> u64 {
    self.level_per_token_id.get(&token_id).unwrap()
  }

  fn increment_token_id(
    &mut self,
  ) {
    let token_id_num: u64 = self.current_token_id.parse().unwrap(); 
    let token_id_increment: u64 = &token_id_num + 1;
    self.current_token_id = token_id_increment.to_string();
  }

  fn get_metadata_per_type(
    &self,
    metadata_type: u64,
    metadata_set: u64,
  ) -> TokenMetadata {
    let current = env::block_timestamp() / 1000000;
    let next = current + 300000;

    let mut metadata = match metadata_type {
      0 => metadatas::get_metadata_egg(metadata_set),
      1 => metadatas::get_metadata_monster_1(metadata_set),
      2 => metadatas::get_metadata_monster_2(metadata_set),
      3 => metadatas::get_metadata_monster_3(metadata_set),
      _ => panic!("You have reach the maximum level of your monster"),
    };

    metadata.issued_at = Some(current.to_string());
    metadata.copies = Some(1u64);
    metadata.extra = Some(next.to_string());

    metadata
  }

  fn get_random_number(&self) -> u8 {
    let rand: u8 = *env::random_seed().get(0).unwrap();
    rand
  }

}

#[near_bindgen]
impl NonFungibleTokenMetadataProvider for Contract {
    fn nft_metadata(&self) -> NFTContractMetadata {
        self.metadata.get().unwrap()
    }
}

fn refund_deposit() {
  // let required_cost = env::storage_byte_cost() * Balance::from(storage_used);
  let required_cost = MINIMUM_EARLY_DEPOSIT;

  let attached_deposit = env::attached_deposit();

  assert!(
    required_cost <= attached_deposit,
    "Must attach 1 NEAR to cover storage",
  );

  let refund = attached_deposit - required_cost;

  if refund > 1 {
    Promise::new(env::predecessor_account_id()).transfer(refund);
  }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use near_sdk::test_utils::{accounts, VMContextBuilder};
    use near_sdk::MockedBlockchain;
    use near_sdk::{testing_env};

    const DATA_IMAGE_SVG_PARAS_ICON: &str = "data:image/svg+xml,%3Csvg width='1080' height='1080' viewBox='0 0 1080 1080' fill='none' xmlns='http://www.w3.org/2000/svg'%3E%3Crect width='1080' height='1080' rx='10' fill='%230000BA'/%3E%3Cpath fill-rule='evenodd' clip-rule='evenodd' d='M335.238 896.881L240 184L642.381 255.288C659.486 259.781 675.323 263.392 689.906 266.718C744.744 279.224 781.843 287.684 801.905 323.725C827.302 369.032 840 424.795 840 491.014C840 557.55 827.302 613.471 801.905 658.779C776.508 704.087 723.333 726.74 642.381 726.74H468.095L501.429 896.881H335.238ZM387.619 331.329L604.777 369.407C614.008 371.807 622.555 373.736 630.426 375.513C660.02 382.193 680.042 386.712 690.869 405.963C704.575 430.164 711.428 459.95 711.428 495.321C711.428 530.861 704.575 560.731 690.869 584.932C677.163 609.133 648.466 621.234 604.777 621.234H505.578L445.798 616.481L387.619 331.329Z' fill='white'/%3E%3C/svg%3E";

    fn get_context(predecessor_account_id: ValidAccountId) -> VMContextBuilder {
        let mut builder = VMContextBuilder::new();
        builder
            .current_account_id(accounts(0))
            .signer_account_id(predecessor_account_id.clone())
            .predecessor_account_id(predecessor_account_id);
        builder
    }

    fn setup_contract() -> (VMContextBuilder, Contract) {
        let mut context = VMContextBuilder::new();
        testing_env!(context.predecessor_account_id(accounts(0)).build());
        let contract = Contract::new_default_meta(accounts(0));
        (context, contract)
    }

    fn sample_token_metadata() -> TokenMetadata {
      TokenMetadata {
        title: Some("Olympus Mons".into()),
        description: Some("The tallest mountain in the charted solar system".into()),
        media: None,
        media_hash: None,
        copies: Some(1u64),
        issued_at: None,
        expires_at: None,
        starts_at: None,
        updated_at: None,
        extra: None,
        reference: None,
        reference_hash: None,
      }
    }

    #[test]
    fn test_new() {
        let mut context = get_context(accounts(1));
        testing_env!(context.build());
        let contract = Contract::new(
            accounts(1),
            NFTContractMetadata {
                spec: NFT_METADATA_SPEC.to_string(),
                name: "Triple Triad".to_string(),
                symbol: "TRIAD".to_string(),
                icon: Some(DATA_IMAGE_SVG_PARAS_ICON.to_string()),
                base_uri: Some("https://ipfs.fleek.co/ipfs/".to_string()),
                reference: None,
                reference_hash: None,
            }
        );
        testing_env!(context.is_view(true).build());
        assert_eq!(contract.get_owner(), accounts(1).to_string());
        assert_eq!(contract.nft_metadata().base_uri.unwrap(), "https://ipfs.fleek.co/ipfs/".to_string());
        assert_eq!(contract.nft_metadata().icon.unwrap(), DATA_IMAGE_SVG_PARAS_ICON.to_string());
    }
}