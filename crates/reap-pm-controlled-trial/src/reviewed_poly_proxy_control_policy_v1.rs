//! Exact offline structural policy for a Polymarket type-1 proxy relationship.
//!
//! This additive policy freezes public deterministic constants and the proof
//! grammar that a future online actor would have to satisfy. It does not prove
//! that a proxy is deployed, that current Polygon or exchange state matches
//! these constants, that a signer possesses a key, or that any signer has
//! exclusive control. The official-source manifest already frozen by PM-T2
//! does not pin the source bytes or deployed correspondence needed for those
//! claims, so every such status is closed and required-but-unavailable.
//!
//! The deterministic CREATE2 relationship is structural only. In particular,
//! its salt hashes the exact packed 20-byte signer address; hashing a
//! left-zero-padded 32-byte ABI address produces a different salt and address.
//! The clone's `_OWNER_SLOT` is an arbitrary exact literal observed in
//! `ProxyWalletLib`; it is not derived from a storage-slot preimage. The
//! adjacent source comment `keccak256("owner")` is mathematically false:
//! Keccak-256 of the UTF-8 bytes `owner` is
//! `0x02016836a56b71f0d02689e69e326f4f4c1b9057164ef592671cf0d37c8040c0`,
//! not the frozen `0x734a...cc20` literal. V1 records that discrepancy as an
//! unauthenticated source-comment error and never treats the comment as proof.
//! The clone is initialized by the factory, so a future storage proof at the
//! exact literal slot must decode to the exact factory, not the signer. Signer
//! authorization is mediated by the exchange's frozen factory/implementation
//! snapshot, type-1 signature recovery, and `getProxyWalletAddress(signer)`.
//! Legacy factory/implementation governance provides additional control paths
//! and permanently prevents this policy from asserting exclusive control.
//!
//! A future positive proof needs exact publisher-authenticated source bytes,
//! deployed-code-to-source correspondence, one trusted finalized Polygon
//! header/state root, locally verified account/code/storage MPT proofs,
//! factory governance and module state, proxy code and exact-literal owner
//! storage, selected-exchange code and getter results, an actor-bound fresh
//! signer challenge, and explicit freshness/reorg handling. No caller-supplied
//! RPC result, clock, source label, digest, or provider statement is accepted
//! here as that proof.
//!
//! This module has no network/RPC client, clock, filesystem writer, binder,
//! token, permit, authentication material, or mutation authority. Its
//! verification is permanently DENIED with a place-dispatch allowance of 0.

use std::{fmt, path::Path};

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest as _, Sha256};
use thiserror::Error;

use crate::{
    OfflineAuthorizationState,
    protected_file::{ProtectedFileKind, read_one},
};

pub const REVIEWED_POLY_PROXY_CONTROL_POLICY_V1_SCHEMA_VERSION: u32 = 1;
pub const PM_T2_REVIEWED_POLY_PROXY_CONTROL_POLICY_FILE_V1: &str =
    "pm-t2-reviewed-poly-proxy-control-policy-v1.json";

pub const PM_POLY_PROXY_POLYGON_CHAIN_ID_V1: u64 = 137;
pub const PM_POLY_PROXY_FACTORY_ADDRESS_V1: &str = "0xaB45c5A4B0c941a2F231C04C3f49182e1A254052";
pub const PM_POLY_PROXY_IMPLEMENTATION_ADDRESS_V1: &str =
    "0x44e999d5c2F66Ef0861317f9A4805AC2e90aEB4f";
pub const PM_POLY_PROXY_STANDARD_V2_EXCHANGE_ADDRESS_V1: &str =
    "0xE111180000d2663C0091e4f400237545B87B996B";
pub const PM_POLY_PROXY_NEGATIVE_RISK_V2_EXCHANGE_ADDRESS_V1: &str =
    "0xe2222d279d744050d28e00520010520000310F59";
pub const PM_POLY_PROXY_INIT_CODE_BYTE_LENGTH_V1: u16 = 167;
pub const PM_POLY_PROXY_INIT_CODE_KECCAK256_V1: &str =
    "d21df8dc65880a8606f09fe0ce3df9b8869287ab0b058be05aa9e8af6330a00b";
pub const PM_POLY_PROXY_INIT_CODE_HEX_V1: &str = concat!(
    "3d3d606380380380913d393d73",
    "ab45c5a4b0c941a2f231c04c3f49182e1a254052",
    "5af4602a57600080fd5b602d8060366000396000f3363d3d373d3d3d363d73",
    "44e999d5c2f66ef0861317f9a4805ac2e90aeb4f",
    "5af43d82803e903d91602b57fd5bf3",
    "52e831dd",
    "0000000000000000000000000000000000000000000000000000000000000020",
    "0000000000000000000000000000000000000000000000000000000000000000",
);
pub const PM_POLY_PROXY_RUNTIME_BYTE_LENGTH_V1: u16 = 45;
pub const PM_POLY_PROXY_RUNTIME_KECCAK256_V1: &str =
    "2fba6fc187f77826faf197b5508d25c98b54581c5123333f1060f2bd87f38b9b";
pub const PM_POLY_PROXY_RUNTIME_HEX_V1: &str = concat!(
    "363d3d373d3d3d363d73",
    "44e999d5c2f66ef0861317f9a4805ac2e90aeb4f",
    "5af43d82803e903d91602b57fd5bf3",
);
pub const PM_POLY_PROXY_OWNER_STORAGE_SLOT_LITERAL_V1: &str =
    "0x734a2a5caf82146a5ddd5263d9af379f9f72724959f0567ddc9df2c40cf2cc20";
pub const PM_POLY_PROXY_OWNER_UTF8_KECCAK256_V1: &str =
    "0x02016836a56b71f0d02689e69e326f4f4c1b9057164ef592671cf0d37c8040c0";

const MAX_CANONICAL_REVIEWED_POLY_PROXY_CONTROL_POLICY_BYTES_V1: usize = 128 * 1024;
const REVIEWED_POLY_PROXY_CONTROL_POLICY_V1_FINGERPRINT_DOMAIN: &[u8] =
    b"reap.pm-t2.controlled-trial.reviewed-poly-proxy-control-policy.v1\0";

const SOURCE_OBSERVED_DATE_V1: &str = "2026-08-11";
const CTF_EXCHANGE_V2_COMMIT_V1: &str = "ccc0596074f4dfd62c944fbca4de252893b82b4b";
const CTF_SIGNATURES_PATH_V1: &str = "src/exchange/mixins/Signatures.sol";
const CTF_POLY_FACTORY_HELPER_PATH_V1: &str = "src/exchange/mixins/PolyFactoryHelper.sol";
const PROXY_FACTORIES_COMMIT_V1: &str = "7137c021e6954d671095f77c94afc3d083d10a84";
const TS_SDK_COMMIT_V1: &str = "e19e87da2670a7162ddce1674870fece4521d196";
const MAGIC_EXAMPLE_COMMIT_V1: &str = "a8cd96a7bfe725c085bc7d4f882519b60e4e6f05";
const GET_PROXY_WALLET_ADDRESS_SIGNATURE_V1: &str = "getProxyWalletAddress(address)";
const GET_PROXY_FACTORY_SIGNATURE_V1: &str = "getProxyFactory()";
const GET_PROXY_IMPLEMENTATION_SIGNATURE_V1: &str = "getProxyImplementation()";
const INITIALIZER_SELECTOR_HEX_V1: &str = "52e831dd";

/// The only role this static record can have.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewedPolyProxyRecordRoleV1 {
    NonAuthorizingStructuralPolicyOnlyV1,
}

/// The exact signer-to-CREATE2-salt relationship admitted by V1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewedPolyProxySignerSaltRelationV1 {
    Keccak256ExactAbiEncodePackedTwentyByteSignerV1,
}

/// The exact CREATE2 output relationship admitted by V1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewedPolyProxyCreate2AddressRelationV1 {
    LowTwentyBytesOfKeccak256FfFactorySaltInitCodeHashV1,
}

/// Fixed initializer tail of the 167-byte creation code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewedPolyProxyInitializerEncodingV1 {
    SelectorThenAbiEncodeSingleEmptyBytesValueV1,
}

/// The exact deployed runtime family admitted structurally by V1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewedPolyProxyRuntimeRelationV1 {
    Eip1167DelegateProxyToExactImplementationV1,
}

/// Required interpretation of the exact owner storage word.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewedPolyProxyStoredOwnerRelationV1 {
    ExactFactoryAddressBecauseFactoryCallsInitializeV1,
}

/// The storage slot is frozen as a source literal, not a derived hash.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewedPolyProxyOwnerStorageSlotSourceV1 {
    ExactLiteralFromProxyWalletLibV1,
}

/// Closed label for the unauthenticated, mathematically incorrect adjacent
/// source comment. It grants no source-authorship or correspondence claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewedPolyProxyOwnerStorageSlotCommentStatusV1 {
    UnauthenticatedSourceCommentErrorKeccak256OwnerIsFalseV1,
}

/// Structural type-0 discrimination; it grants no type-0 authority here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewedPolyProxyTypeZeroRelationV1 {
    EoaMakerEqualsRecoveredSignerV1,
}

/// Structural type-1 discrimination required by the selected exchange.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewedPolyProxyTypeOneRelationV1 {
    NonemptyEcdsaRecoveredSignerAndExchangeDerivedMakerV1,
}

/// Type 2/3 are outside this policy because their exact pins are unavailable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewedPolyProxyOtherSignatureTypesStatusV1 {
    NotAdmittedWithoutSeparateExactReviewedPinsV1,
}

/// One closed fail-safe state used for every missing positive proof class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewedPolyProxyRequiredEvidenceStatusV1 {
    RequiredButUnavailableV1,
}

/// A future integration must satisfy every field carrying this value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewedPolyProxyFutureProofRequirementV1 {
    RequiredV1,
}

/// A deterministic address computation is never current control evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewedPolyProxyDeterministicRelationLimitationV1 {
    StructuralOnlyNoDeploymentStateOrControlClaimV1,
}

/// Legacy governance makes an exclusive-control statement unavailable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewedPolyProxyExclusiveControlLimitationV1 {
    LegacyGovernanceAndModulesPreventExclusiveControlSemanticsV1,
}

/// Public commit/path labels observed during review. They are not loaded,
/// publisher-authenticated, or part of the frozen official-source manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedPolyProxyUnattestedSourceLabelsV1 {
    pub observed_date_utc: String,
    pub ctf_exchange_v2_commit: String,
    pub ctf_signatures_path: String,
    pub ctf_poly_factory_helper_path: String,
    pub proxy_factories_commit: String,
    pub ts_sdk_commit: String,
    pub magic_example_commit: String,
}

impl ReviewedPolyProxyUnattestedSourceLabelsV1 {
    fn validate(&self) -> Result<(), PmReviewedPolyProxyControlPolicyV1Error> {
        if self.observed_date_utc != SOURCE_OBSERVED_DATE_V1
            || self.ctf_exchange_v2_commit != CTF_EXCHANGE_V2_COMMIT_V1
            || self.ctf_signatures_path != CTF_SIGNATURES_PATH_V1
            || self.ctf_poly_factory_helper_path != CTF_POLY_FACTORY_HELPER_PATH_V1
            || self.proxy_factories_commit != PROXY_FACTORIES_COMMIT_V1
            || self.ts_sdk_commit != TS_SDK_COMMIT_V1
            || self.magic_example_commit != MAGIC_EXAMPLE_COMMIT_V1
        {
            return Err(invalid(
                "reviewed Poly proxy source labels differ from the exact unattested V1 labels",
            ));
        }
        Ok(())
    }
}

/// Exact fixed creation code and its independently reviewed Keccak label.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedPolyProxyInitCodeV1 {
    pub byte_length: u16,
    pub hex: String,
    pub keccak256: String,
    pub initializer_selector_hex: String,
    pub initializer_encoding: ReviewedPolyProxyInitializerEncodingV1,
}

impl ReviewedPolyProxyInitCodeV1 {
    fn validate(&self) -> Result<(), PmReviewedPolyProxyControlPolicyV1Error> {
        if self.byte_length != PM_POLY_PROXY_INIT_CODE_BYTE_LENGTH_V1
            || self.hex != PM_POLY_PROXY_INIT_CODE_HEX_V1
            || self.hex.len() != usize::from(self.byte_length) * 2
            || self.keccak256 != PM_POLY_PROXY_INIT_CODE_KECCAK256_V1
            || self.initializer_selector_hex != INITIALIZER_SELECTOR_HEX_V1
            || self.initializer_encoding
                != ReviewedPolyProxyInitializerEncodingV1::SelectorThenAbiEncodeSingleEmptyBytesValueV1
        {
            return Err(invalid(
                "reviewed Poly proxy exact init-code relation drifted",
            ));
        }
        Ok(())
    }
}

/// Exact fixed 45-byte delegate-proxy runtime template.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedPolyProxyRuntimeTemplateV1 {
    pub byte_length: u16,
    pub hex: String,
    pub keccak256: String,
    pub relation: ReviewedPolyProxyRuntimeRelationV1,
}

impl ReviewedPolyProxyRuntimeTemplateV1 {
    fn validate(&self) -> Result<(), PmReviewedPolyProxyControlPolicyV1Error> {
        if self.byte_length != PM_POLY_PROXY_RUNTIME_BYTE_LENGTH_V1
            || self.hex != PM_POLY_PROXY_RUNTIME_HEX_V1
            || self.hex.len() != usize::from(self.byte_length) * 2
            || self.keccak256 != PM_POLY_PROXY_RUNTIME_KECCAK256_V1
            || self.relation
                != ReviewedPolyProxyRuntimeRelationV1::Eip1167DelegateProxyToExactImplementationV1
        {
            return Err(invalid(
                "reviewed Poly proxy exact runtime-template relation drifted",
            ));
        }
        Ok(())
    }
}

/// Exact deterministic-address grammar. It accepts no concrete signer or
/// proxy address and therefore proves no account-specific relationship.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedPolyProxyCreate2PolicyV1 {
    pub signer_salt_relation: ReviewedPolyProxySignerSaltRelationV1,
    pub abi_padded_thirty_two_byte_signer_input_forbidden: bool,
    pub address_relation: ReviewedPolyProxyCreate2AddressRelationV1,
    pub init_code: ReviewedPolyProxyInitCodeV1,
}

impl ReviewedPolyProxyCreate2PolicyV1 {
    fn validate(&self) -> Result<(), PmReviewedPolyProxyControlPolicyV1Error> {
        if self.signer_salt_relation
            != ReviewedPolyProxySignerSaltRelationV1::Keccak256ExactAbiEncodePackedTwentyByteSignerV1
            || !self.abi_padded_thirty_two_byte_signer_input_forbidden
            || self.address_relation
                != ReviewedPolyProxyCreate2AddressRelationV1::LowTwentyBytesOfKeccak256FfFactorySaltInitCodeHashV1
        {
            return Err(invalid(
                "reviewed Poly proxy CREATE2/salt policy drifted",
            ));
        }
        self.init_code.validate()
    }
}

/// Exact storage-slot interpretation required of a future finalized proof.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedPolyProxyOwnerStoragePolicyV1 {
    pub slot_literal: String,
    pub slot_source: ReviewedPolyProxyOwnerStorageSlotSourceV1,
    pub upstream_inline_comment_status: ReviewedPolyProxyOwnerStorageSlotCommentStatusV1,
    pub actual_keccak256_of_utf8_owner: String,
    pub slot_literal_differs_from_actual_keccak256_of_utf8_owner: bool,
    pub stored_owner_relation: ReviewedPolyProxyStoredOwnerRelationV1,
    pub decoded_owner_must_equal_factory_address: String,
}

impl ReviewedPolyProxyOwnerStoragePolicyV1 {
    fn validate(&self) -> Result<(), PmReviewedPolyProxyControlPolicyV1Error> {
        if self.slot_literal != PM_POLY_PROXY_OWNER_STORAGE_SLOT_LITERAL_V1
            || self.slot_source
                != ReviewedPolyProxyOwnerStorageSlotSourceV1::ExactLiteralFromProxyWalletLibV1
            || self.upstream_inline_comment_status
                != ReviewedPolyProxyOwnerStorageSlotCommentStatusV1::UnauthenticatedSourceCommentErrorKeccak256OwnerIsFalseV1
            || self.actual_keccak256_of_utf8_owner != PM_POLY_PROXY_OWNER_UTF8_KECCAK256_V1
            || !self.slot_literal_differs_from_actual_keccak256_of_utf8_owner
            || self.stored_owner_relation
                != ReviewedPolyProxyStoredOwnerRelationV1::ExactFactoryAddressBecauseFactoryCallsInitializeV1
            || self.decoded_owner_must_equal_factory_address != PM_POLY_PROXY_FACTORY_ADDRESS_V1
        {
            return Err(invalid(
                "reviewed Poly proxy owner-storage policy drifted",
            ));
        }
        Ok(())
    }
}

/// Fixed public chain, factory, implementation, CREATE2, runtime, and owner
/// relations. These are requirements, not observations of live state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedPolyProxyStructuralPolicyV1 {
    pub polygon_chain_id: u64,
    pub factory_address: String,
    pub implementation_address: String,
    pub create2: ReviewedPolyProxyCreate2PolicyV1,
    pub runtime_template: ReviewedPolyProxyRuntimeTemplateV1,
    pub owner_storage: ReviewedPolyProxyOwnerStoragePolicyV1,
}

impl ReviewedPolyProxyStructuralPolicyV1 {
    fn validate(&self) -> Result<(), PmReviewedPolyProxyControlPolicyV1Error> {
        if self.polygon_chain_id != PM_POLY_PROXY_POLYGON_CHAIN_ID_V1
            || self.factory_address != PM_POLY_PROXY_FACTORY_ADDRESS_V1
            || self.implementation_address != PM_POLY_PROXY_IMPLEMENTATION_ADDRESS_V1
        {
            return Err(invalid(
                "reviewed Poly proxy chain, factory, or implementation drifted",
            ));
        }
        self.create2.validate()?;
        self.runtime_template.validate()?;
        self.owner_storage.validate()
    }
}

/// Closed selected-exchange/type-discrimination requirements.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedPolyProxyExchangeOraclePolicyV1 {
    pub standard_v2_exchange_address: String,
    pub negative_risk_v2_exchange_address: String,
    pub selected_signature_type: u8,
    pub type_zero_relation: ReviewedPolyProxyTypeZeroRelationV1,
    pub type_one_relation: ReviewedPolyProxyTypeOneRelationV1,
    pub type_two_and_three_status: ReviewedPolyProxyOtherSignatureTypesStatusV1,
    pub type_one_signature_must_be_nonempty: bool,
    pub ecdsa_recovered_address_must_equal_declared_signer: bool,
    pub get_proxy_wallet_address_signature: String,
    pub get_proxy_factory_signature: String,
    pub get_proxy_implementation_signature: String,
    pub get_proxy_wallet_address_of_signer_must_equal_maker: bool,
    pub selected_exchange_factory_getter_must_equal_exact_factory: bool,
    pub selected_exchange_implementation_getter_must_equal_exact_implementation: bool,
}

impl ReviewedPolyProxyExchangeOraclePolicyV1 {
    fn validate(&self) -> Result<(), PmReviewedPolyProxyControlPolicyV1Error> {
        if self.standard_v2_exchange_address != PM_POLY_PROXY_STANDARD_V2_EXCHANGE_ADDRESS_V1
            || self.negative_risk_v2_exchange_address
                != PM_POLY_PROXY_NEGATIVE_RISK_V2_EXCHANGE_ADDRESS_V1
            || self.selected_signature_type != 1
            || self.type_zero_relation
                != ReviewedPolyProxyTypeZeroRelationV1::EoaMakerEqualsRecoveredSignerV1
            || self.type_one_relation
                != ReviewedPolyProxyTypeOneRelationV1::NonemptyEcdsaRecoveredSignerAndExchangeDerivedMakerV1
            || self.type_two_and_three_status
                != ReviewedPolyProxyOtherSignatureTypesStatusV1::NotAdmittedWithoutSeparateExactReviewedPinsV1
            || !self.type_one_signature_must_be_nonempty
            || !self.ecdsa_recovered_address_must_equal_declared_signer
            || self.get_proxy_wallet_address_signature != GET_PROXY_WALLET_ADDRESS_SIGNATURE_V1
            || self.get_proxy_factory_signature != GET_PROXY_FACTORY_SIGNATURE_V1
            || self.get_proxy_implementation_signature != GET_PROXY_IMPLEMENTATION_SIGNATURE_V1
            || !self.get_proxy_wallet_address_of_signer_must_equal_maker
            || !self.selected_exchange_factory_getter_must_equal_exact_factory
            || !self.selected_exchange_implementation_getter_must_equal_exact_implementation
        {
            return Err(invalid(
                "reviewed Poly proxy selected-exchange/type oracle policy drifted",
            ));
        }
        Ok(())
    }
}

/// All missing positive evidence classes are closed and unavailable. There is
/// no digest, key, signature, endpoint, block, proof, signer, maker, or proxy
/// value through which a caller can claim one is present.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedPolyProxyUnavailableEvidenceV1 {
    pub exact_source_bytes_and_publisher_authorship: ReviewedPolyProxyRequiredEvidenceStatusV1,
    pub deployed_code_source_correspondence: ReviewedPolyProxyRequiredEvidenceStatusV1,
    pub authenticated_finalized_polygon_header_and_state_root:
        ReviewedPolyProxyRequiredEvidenceStatusV1,
    pub locally_verified_account_code_and_storage_mpt_bundle:
        ReviewedPolyProxyRequiredEvidenceStatusV1,
    pub current_factory_governance_and_module_state: ReviewedPolyProxyRequiredEvidenceStatusV1,
    pub current_proxy_code_and_owner_state: ReviewedPolyProxyRequiredEvidenceStatusV1,
    pub current_selected_exchange_code_and_getter_results:
        ReviewedPolyProxyRequiredEvidenceStatusV1,
    pub actor_bound_fresh_signer_challenge: ReviewedPolyProxyRequiredEvidenceStatusV1,
    pub provider_authorship_and_non_equivocation: ReviewedPolyProxyRequiredEvidenceStatusV1,
    pub proof_freshness_and_reorg_monitoring: ReviewedPolyProxyRequiredEvidenceStatusV1,
}

impl ReviewedPolyProxyUnavailableEvidenceV1 {
    fn validate(&self) -> Result<(), PmReviewedPolyProxyControlPolicyV1Error> {
        let statuses = [
            self.exact_source_bytes_and_publisher_authorship,
            self.deployed_code_source_correspondence,
            self.authenticated_finalized_polygon_header_and_state_root,
            self.locally_verified_account_code_and_storage_mpt_bundle,
            self.current_factory_governance_and_module_state,
            self.current_proxy_code_and_owner_state,
            self.current_selected_exchange_code_and_getter_results,
            self.actor_bound_fresh_signer_challenge,
            self.provider_authorship_and_non_equivocation,
            self.proof_freshness_and_reorg_monitoring,
        ];
        if statuses.into_iter().any(|status| {
            status != ReviewedPolyProxyRequiredEvidenceStatusV1::RequiredButUnavailableV1
        }) {
            return Err(invalid(
                "reviewed Poly proxy missing-evidence status is not closed-unavailable",
            ));
        }
        Ok(())
    }
}

/// Exact proof work required before another, separately reviewed runtime layer
/// could make a positive, account-specific, nonexclusive control claim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedPolyProxyFutureProofPolicyV1 {
    pub exact_source_bytes_and_publisher_authorship: ReviewedPolyProxyFutureProofRequirementV1,
    pub deployed_code_source_correspondence: ReviewedPolyProxyFutureProofRequirementV1,
    pub trusted_finalized_polygon_header_and_state_root: ReviewedPolyProxyFutureProofRequirementV1,
    pub same_state_root_for_all_account_and_storage_proofs:
        ReviewedPolyProxyFutureProofRequirementV1,
    pub locally_verified_account_mpt_proofs: ReviewedPolyProxyFutureProofRequirementV1,
    pub locally_verified_code_bytes_and_keccak: ReviewedPolyProxyFutureProofRequirementV1,
    pub locally_verified_storage_mpt_proofs: ReviewedPolyProxyFutureProofRequirementV1,
    pub factory_governance_owner_and_module_state: ReviewedPolyProxyFutureProofRequirementV1,
    pub proxy_runtime_code_and_exact_implementation: ReviewedPolyProxyFutureProofRequirementV1,
    pub exact_literal_proxy_owner_slot_storage_proof_equals_factory:
        ReviewedPolyProxyFutureProofRequirementV1,
    pub selected_exchange_code_factory_and_implementation_getters:
        ReviewedPolyProxyFutureProofRequirementV1,
    pub selected_exchange_get_proxy_wallet_address_call: ReviewedPolyProxyFutureProofRequirementV1,
    pub type_one_nonempty_signature_and_recovered_signer: ReviewedPolyProxyFutureProofRequirementV1,
    pub actor_bound_fresh_signer_possession_challenge: ReviewedPolyProxyFutureProofRequirementV1,
    pub authenticated_provider_and_non_equivocation: ReviewedPolyProxyFutureProofRequirementV1,
    pub bounded_observation_freshness: ReviewedPolyProxyFutureProofRequirementV1,
    pub finalized_reorg_detection_and_invalidation: ReviewedPolyProxyFutureProofRequirementV1,
}

impl ReviewedPolyProxyFutureProofPolicyV1 {
    fn validate(&self) -> Result<(), PmReviewedPolyProxyControlPolicyV1Error> {
        let requirements = [
            self.exact_source_bytes_and_publisher_authorship,
            self.deployed_code_source_correspondence,
            self.trusted_finalized_polygon_header_and_state_root,
            self.same_state_root_for_all_account_and_storage_proofs,
            self.locally_verified_account_mpt_proofs,
            self.locally_verified_code_bytes_and_keccak,
            self.locally_verified_storage_mpt_proofs,
            self.factory_governance_owner_and_module_state,
            self.proxy_runtime_code_and_exact_implementation,
            self.exact_literal_proxy_owner_slot_storage_proof_equals_factory,
            self.selected_exchange_code_factory_and_implementation_getters,
            self.selected_exchange_get_proxy_wallet_address_call,
            self.type_one_nonempty_signature_and_recovered_signer,
            self.actor_bound_fresh_signer_possession_challenge,
            self.authenticated_provider_and_non_equivocation,
            self.bounded_observation_freshness,
            self.finalized_reorg_detection_and_invalidation,
        ];
        if requirements
            .into_iter()
            .any(|requirement| requirement != ReviewedPolyProxyFutureProofRequirementV1::RequiredV1)
        {
            return Err(invalid(
                "reviewed Poly proxy future proof policy omitted a required conjunct",
            ));
        }
        Ok(())
    }
}

/// Explicit limits on what the structural record can ever mean.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedPolyProxyLimitationsV1 {
    pub deterministic_relation: ReviewedPolyProxyDeterministicRelationLimitationV1,
    pub exclusive_control: ReviewedPolyProxyExclusiveControlLimitationV1,
    pub signer_control_if_future_proven_is_nonexclusive: bool,
    pub deterministic_address_is_not_deployment_evidence: bool,
    pub source_labels_are_not_source_authorship_or_deployed_correspondence: bool,
    pub provider_statement_is_not_a_locally_verified_state_proof: bool,
}

impl ReviewedPolyProxyLimitationsV1 {
    fn validate(&self) -> Result<(), PmReviewedPolyProxyControlPolicyV1Error> {
        if self.deterministic_relation
            != ReviewedPolyProxyDeterministicRelationLimitationV1::StructuralOnlyNoDeploymentStateOrControlClaimV1
            || self.exclusive_control
                != ReviewedPolyProxyExclusiveControlLimitationV1::LegacyGovernanceAndModulesPreventExclusiveControlSemanticsV1
            || !self.signer_control_if_future_proven_is_nonexclusive
            || !self.deterministic_address_is_not_deployment_evidence
            || !self.source_labels_are_not_source_authorship_or_deployed_correspondence
            || !self.provider_statement_is_not_a_locally_verified_state_proof
        {
            return Err(invalid("reviewed Poly proxy limitation policy drifted"));
        }
        Ok(())
    }
}

/// Fully fixed public structural policy. It contains no concrete signer,
/// maker, proxy, block, proof, provider, actor, credential, or request value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedPolyProxyControlPolicyV1 {
    pub schema_version: u32,
    pub record_role: ReviewedPolyProxyRecordRoleV1,
    pub unattested_source_labels: ReviewedPolyProxyUnattestedSourceLabelsV1,
    pub structural_policy: ReviewedPolyProxyStructuralPolicyV1,
    pub exchange_oracle_policy: ReviewedPolyProxyExchangeOraclePolicyV1,
    pub unavailable_evidence: ReviewedPolyProxyUnavailableEvidenceV1,
    pub future_proof_policy: ReviewedPolyProxyFutureProofPolicyV1,
    pub limitations: ReviewedPolyProxyLimitationsV1,
}

impl ReviewedPolyProxyControlPolicyV1 {
    fn validate_intrinsic(&self) -> Result<(), PmReviewedPolyProxyControlPolicyV1Error> {
        if self.schema_version != REVIEWED_POLY_PROXY_CONTROL_POLICY_V1_SCHEMA_VERSION
            || self.record_role
                != ReviewedPolyProxyRecordRoleV1::NonAuthorizingStructuralPolicyOnlyV1
        {
            return Err(invalid(
                "reviewed Poly proxy schema or non-authorizing role drifted",
            ));
        }
        self.unattested_source_labels.validate()?;
        self.structural_policy.validate()?;
        self.exchange_oracle_policy.validate()?;
        self.unavailable_evidence.validate()?;
        self.future_proof_policy.validate()?;
        self.limitations.validate()
    }
}

/// Move-only protected holder. It exposes only canonical hash/length and a
/// domain-separated fingerprint; raw bytes and the parsed value are private.
pub struct CanonicalReviewedPolyProxyControlPolicyV1 {
    value: ReviewedPolyProxyControlPolicyV1,
    canonical_bytes: Vec<u8>,
    canonical_sha256: String,
    fingerprint: String,
}

impl CanonicalReviewedPolyProxyControlPolicyV1 {
    #[must_use]
    pub fn canonical_sha256(&self) -> &str {
        &self.canonical_sha256
    }

    #[must_use]
    pub fn canonical_length(&self) -> u64 {
        self.canonical_bytes.len() as u64
    }

    #[must_use]
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }
}

impl fmt::Debug for CanonicalReviewedPolyProxyControlPolicyV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "CanonicalReviewedPolyProxyControlPolicyV1(<exact-protected-canonical-bytes; no-value-signer-maker-proxy-proof-provider-actor-or-authority-projection; redacted; denied>)",
        )
    }
}

/// Structural display only. Every observation, control, currentness,
/// provider, authority, and dispatch fact is explicitly false.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReviewedPolyProxyControlPolicyVerificationV1 {
    pub schema_version: u32,
    pub reviewed_poly_proxy_control_policy_fingerprint: String,
    pub exact_unattested_source_labels_structurally_valid: bool,
    pub exact_chain_factory_and_implementation_structurally_valid: bool,
    pub exact_packed_signer_salt_and_create2_grammar_structurally_valid: bool,
    pub exact_init_code_and_keccak_label_structurally_valid: bool,
    pub exact_runtime_template_and_keccak_label_structurally_valid: bool,
    pub exact_literal_owner_slot_comment_discrepancy_and_factory_relation_structurally_valid: bool,
    pub exact_exchange_oracle_and_type_discrimination_structurally_valid: bool,
    pub required_unavailable_evidence_statuses_structurally_valid: bool,
    pub future_proof_requirements_structurally_valid: bool,
    pub structural_and_legacy_governance_limitations_valid: bool,
    pub official_source_manifest_pins_proxy_sources: bool,
    pub exact_source_bytes_loaded_and_hash_verified: bool,
    pub source_publisher_authorship_attested: bool,
    pub upstream_owner_slot_inline_comment_authenticated: bool,
    pub owner_slot_literal_equals_keccak256_utf8_owner: bool,
    pub deployed_code_source_correspondence_verified: bool,
    pub current_polygon_chain_id_observed: bool,
    pub finalized_polygon_header_observed: bool,
    pub finalized_polygon_header_authenticated: bool,
    pub finalized_polygon_state_root_trusted: bool,
    pub finalized_polygon_state_rechecked: bool,
    pub same_finalized_state_root_used_for_all_proofs: bool,
    pub account_mpt_proofs_locally_verified: bool,
    pub code_bytes_loaded_and_keccak_verified: bool,
    pub storage_mpt_proofs_locally_verified: bool,
    pub factory_account_and_code_verified: bool,
    pub implementation_account_and_code_verified: bool,
    pub factory_governance_owner_verified: bool,
    pub factory_module_state_verified: bool,
    pub live_signer_address_bound: bool,
    pub live_maker_proxy_address_bound: bool,
    pub signer_salt_derived_from_live_signer: bool,
    pub proxy_address_derived_for_live_signer: bool,
    pub proxy_account_deployed: bool,
    pub proxy_runtime_code_verified: bool,
    pub proxy_implementation_verified: bool,
    pub proxy_owner_storage_proof_verified: bool,
    pub proxy_owner_equals_factory_verified: bool,
    pub selected_exchange_domain_bound: bool,
    pub selected_exchange_account_and_code_verified: bool,
    pub selected_exchange_proxy_factory_getter_verified: bool,
    pub selected_exchange_proxy_implementation_getter_verified: bool,
    pub selected_exchange_get_proxy_wallet_address_call_verified: bool,
    pub selected_signature_type_one_checked: bool,
    pub type_one_signature_nonempty_checked: bool,
    pub type_one_ecdsa_recovered_signer_checked: bool,
    pub type_one_derived_proxy_equals_maker_checked: bool,
    pub type_two_and_three_exclusion_checked: bool,
    pub actor_generation_bound: bool,
    pub fresh_signer_challenge_issued: bool,
    pub fresh_signer_challenge_signature_verified: bool,
    pub signer_private_key_possession_attested: bool,
    pub signer_controls_proxy_nonexclusively_attested: bool,
    pub signer_exclusively_controls_proxy_attested: bool,
    pub factory_governance_exclusivity_absence_attested: bool,
    pub signer_proxy_relationship_current_attested: bool,
    pub signer_proxy_relationship_unrevoked_attested: bool,
    pub factory_and_implementation_current_attested: bool,
    pub provider_trust_root_authenticated: bool,
    pub provider_authorship_attested: bool,
    pub provider_non_equivocation_attested: bool,
    pub source_owned_current_time_checked: bool,
    pub proof_observation_freshness_checked: bool,
    pub finalized_reorg_monitoring_active: bool,
    pub stale_or_reorged_proof_invalidation_checked: bool,
    pub live_positive_proxy_control_proof_complete: bool,
    pub proxy_control_binder_constructed: bool,
    pub proxy_control_token_or_permit_minted: bool,
    pub credential_mutation_authority_attested: bool,
    pub authenticated_request_constructed: bool,
    pub signed_order_body_constructed: bool,
    pub place_dispatch_owner_or_grant_minted: bool,
    pub network_or_rpc_dispatch_performed: bool,
    #[serde(flatten)]
    pub authorization: OfflineAuthorizationState,
}

/// Load one exact protected canonical structural policy.
pub fn load_canonical_reviewed_poly_proxy_control_policy_v1(
    path: &Path,
) -> Result<CanonicalReviewedPolyProxyControlPolicyV1, PmReviewedPolyProxyControlPolicyV1Error> {
    let bytes = read_one(
        path,
        ProtectedFileKind::ReviewedPolyProxyControlPolicyV1,
        MAX_CANONICAL_REVIEWED_POLY_PROXY_CONTROL_POLICY_BYTES_V1,
    )
    .map_err(|_| {
        invalid("reviewed Poly proxy control policy protection or stability check failed")
    })?;
    let value: ReviewedPolyProxyControlPolicyV1 = parse_exact_canonical(&bytes)?;
    value.validate_intrinsic()?;
    let canonical_bytes = bytes.to_vec();
    Ok(CanonicalReviewedPolyProxyControlPolicyV1 {
        canonical_sha256: hash_bytes(&[], &canonical_bytes),
        fingerprint: hash_bytes(
            REVIEWED_POLY_PROXY_CONTROL_POLICY_V1_FINGERPRINT_DOMAIN,
            &canonical_bytes,
        ),
        canonical_bytes,
        value,
    })
}

/// Verify fixed structural grammar only, without any source, chain, signer,
/// provider, clock, actor, authentication, or mutation capability.
pub fn verify_reviewed_poly_proxy_control_policy_v1(
    reviewed: &CanonicalReviewedPolyProxyControlPolicyV1,
) -> Result<ReviewedPolyProxyControlPolicyVerificationV1, PmReviewedPolyProxyControlPolicyV1Error> {
    reviewed.value.validate_intrinsic()?;
    Ok(ReviewedPolyProxyControlPolicyVerificationV1 {
        schema_version: reviewed.value.schema_version,
        reviewed_poly_proxy_control_policy_fingerprint: reviewed.fingerprint.clone(),
        exact_unattested_source_labels_structurally_valid: true,
        exact_chain_factory_and_implementation_structurally_valid: true,
        exact_packed_signer_salt_and_create2_grammar_structurally_valid: true,
        exact_init_code_and_keccak_label_structurally_valid: true,
        exact_runtime_template_and_keccak_label_structurally_valid: true,
        exact_literal_owner_slot_comment_discrepancy_and_factory_relation_structurally_valid: true,
        exact_exchange_oracle_and_type_discrimination_structurally_valid: true,
        required_unavailable_evidence_statuses_structurally_valid: true,
        future_proof_requirements_structurally_valid: true,
        structural_and_legacy_governance_limitations_valid: true,
        official_source_manifest_pins_proxy_sources: false,
        exact_source_bytes_loaded_and_hash_verified: false,
        source_publisher_authorship_attested: false,
        upstream_owner_slot_inline_comment_authenticated: false,
        owner_slot_literal_equals_keccak256_utf8_owner: false,
        deployed_code_source_correspondence_verified: false,
        current_polygon_chain_id_observed: false,
        finalized_polygon_header_observed: false,
        finalized_polygon_header_authenticated: false,
        finalized_polygon_state_root_trusted: false,
        finalized_polygon_state_rechecked: false,
        same_finalized_state_root_used_for_all_proofs: false,
        account_mpt_proofs_locally_verified: false,
        code_bytes_loaded_and_keccak_verified: false,
        storage_mpt_proofs_locally_verified: false,
        factory_account_and_code_verified: false,
        implementation_account_and_code_verified: false,
        factory_governance_owner_verified: false,
        factory_module_state_verified: false,
        live_signer_address_bound: false,
        live_maker_proxy_address_bound: false,
        signer_salt_derived_from_live_signer: false,
        proxy_address_derived_for_live_signer: false,
        proxy_account_deployed: false,
        proxy_runtime_code_verified: false,
        proxy_implementation_verified: false,
        proxy_owner_storage_proof_verified: false,
        proxy_owner_equals_factory_verified: false,
        selected_exchange_domain_bound: false,
        selected_exchange_account_and_code_verified: false,
        selected_exchange_proxy_factory_getter_verified: false,
        selected_exchange_proxy_implementation_getter_verified: false,
        selected_exchange_get_proxy_wallet_address_call_verified: false,
        selected_signature_type_one_checked: false,
        type_one_signature_nonempty_checked: false,
        type_one_ecdsa_recovered_signer_checked: false,
        type_one_derived_proxy_equals_maker_checked: false,
        type_two_and_three_exclusion_checked: false,
        actor_generation_bound: false,
        fresh_signer_challenge_issued: false,
        fresh_signer_challenge_signature_verified: false,
        signer_private_key_possession_attested: false,
        signer_controls_proxy_nonexclusively_attested: false,
        signer_exclusively_controls_proxy_attested: false,
        factory_governance_exclusivity_absence_attested: false,
        signer_proxy_relationship_current_attested: false,
        signer_proxy_relationship_unrevoked_attested: false,
        factory_and_implementation_current_attested: false,
        provider_trust_root_authenticated: false,
        provider_authorship_attested: false,
        provider_non_equivocation_attested: false,
        source_owned_current_time_checked: false,
        proof_observation_freshness_checked: false,
        finalized_reorg_monitoring_active: false,
        stale_or_reorged_proof_invalidation_checked: false,
        live_positive_proxy_control_proof_complete: false,
        proxy_control_binder_constructed: false,
        proxy_control_token_or_permit_minted: false,
        credential_mutation_authority_attested: false,
        authenticated_request_constructed: false,
        signed_order_body_constructed: false,
        place_dispatch_owner_or_grant_minted: false,
        network_or_rpc_dispatch_performed: false,
        authorization: OfflineAuthorizationState::DENIED,
    })
}

#[derive(Debug, Error)]
pub enum PmReviewedPolyProxyControlPolicyV1Error {
    #[error("controlled-trial reviewed Poly proxy control policy V1 is invalid: {0}")]
    Invalid(&'static str),
}

fn invalid(message: &'static str) -> PmReviewedPolyProxyControlPolicyV1Error {
    PmReviewedPolyProxyControlPolicyV1Error::Invalid(message)
}

fn parse_exact_canonical<T: DeserializeOwned + Serialize>(
    bytes: &[u8],
) -> Result<T, PmReviewedPolyProxyControlPolicyV1Error> {
    let value: T = serde_json::from_slice(bytes).map_err(|_| {
        invalid("reviewed Poly proxy JSON is malformed, duplicated, unknown, or trailing")
    })?;
    let canonical = serde_json::to_vec(&value)
        .map_err(|_| invalid("reviewed Poly proxy policy cannot be serialized canonically"))?;
    if canonical != bytes {
        return Err(invalid(
            "reviewed Poly proxy bytes are not exact canonical compact JSON",
        ));
    }
    Ok(value)
}

fn hash_bytes(domain: &[u8], bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}
