use reap_pm_core::{EvmAddress, PmErc1155OperatorApproval, U256};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::{PmPolygonChainSourceError, PmPolygonFinalizedBlock};

pub(crate) const CHAIN_ID_REQUEST_ID: u64 = 1;
pub(crate) const FINALIZED_BLOCK_REQUEST_ID: u64 = 2;
pub(crate) const PUSD_ALLOWANCE_REQUEST_ID: u64 = 3;
pub(crate) const CONDITIONAL_TOKENS_APPROVAL_REQUEST_ID: u64 = 4;
pub(crate) const BLOCK_REREAD_REQUEST_ID: u64 = 5;

const JSON_RPC_VERSION: &str = "2.0";
const ETH_CHAIN_ID: &str = "eth_chainId";
const ETH_GET_BLOCK_BY_NUMBER: &str = "eth_getBlockByNumber";
const ETH_CALL: &str = "eth_call";
const FINALIZED_BLOCK_TAG: &str = "finalized";
const ALLOWANCE_SELECTOR: &str = "dd62ed3e";
const IS_APPROVED_FOR_ALL_SELECTOR: &str = "e985e9c5";
const MAX_RPC_ERROR_MESSAGE_BYTES: usize = 512;

#[derive(Serialize)]
struct RpcRequest<P> {
    jsonrpc: &'static str,
    id: u64,
    method: &'static str,
    params: P,
}

#[derive(Serialize)]
struct RpcCall {
    to: String,
    data: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RpcResponse {
    jsonrpc: String,
    id: u64,
    #[serde(default)]
    result: Option<serde_json::Value>,
    #[serde(default)]
    error: Option<RpcErrorPayload>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RpcErrorPayload {
    code: i64,
    message: String,
    #[serde(default)]
    data: Option<serde_json::Value>,
}

// Ethereum block objects contain a version-dependent set of additional
// consensus/execution fields. Those fields are intentionally not accepted as
// evidence. Serde ignores them while requiring and strictly decoding the only
// three fields this cut binds; duplicate bound fields still fail decoding.
#[derive(Deserialize)]
struct RpcBlockResult {
    number: String,
    hash: String,
    timestamp: String,
}

pub(crate) struct ParsedRpcBlock {
    pub(crate) identity: PmPolygonFinalizedBlock,
    pub(crate) canonical_number: String,
}

pub(crate) fn chain_id_request() -> Result<Vec<u8>, PmPolygonChainSourceError> {
    serialize_request(&RpcRequest {
        jsonrpc: JSON_RPC_VERSION,
        id: CHAIN_ID_REQUEST_ID,
        method: ETH_CHAIN_ID,
        params: [(); 0],
    })
}

pub(crate) fn finalized_block_request() -> Result<Vec<u8>, PmPolygonChainSourceError> {
    serialize_request(&RpcRequest {
        jsonrpc: JSON_RPC_VERSION,
        id: FINALIZED_BLOCK_REQUEST_ID,
        method: ETH_GET_BLOCK_BY_NUMBER,
        params: (FINALIZED_BLOCK_TAG, false),
    })
}

pub(crate) fn allowance_request(
    contract: EvmAddress,
    owner: EvmAddress,
    spender: EvmAddress,
    block: &str,
) -> Result<Vec<u8>, PmPolygonChainSourceError> {
    eth_call_request(
        PUSD_ALLOWANCE_REQUEST_ID,
        contract,
        address_pair_calldata(ALLOWANCE_SELECTOR, owner, spender),
        block,
    )
}

pub(crate) fn approval_request(
    contract: EvmAddress,
    owner: EvmAddress,
    spender: EvmAddress,
    block: &str,
) -> Result<Vec<u8>, PmPolygonChainSourceError> {
    eth_call_request(
        CONDITIONAL_TOKENS_APPROVAL_REQUEST_ID,
        contract,
        address_pair_calldata(IS_APPROVED_FOR_ALL_SELECTOR, owner, spender),
        block,
    )
}

pub(crate) fn block_reread_request(block: &str) -> Result<Vec<u8>, PmPolygonChainSourceError> {
    serialize_request(&RpcRequest {
        jsonrpc: JSON_RPC_VERSION,
        id: BLOCK_REREAD_REQUEST_ID,
        method: ETH_GET_BLOCK_BY_NUMBER,
        params: (block, false),
    })
}

fn eth_call_request(
    id: u64,
    contract: EvmAddress,
    data: String,
    block: &str,
) -> Result<Vec<u8>, PmPolygonChainSourceError> {
    serialize_request(&RpcRequest {
        jsonrpc: JSON_RPC_VERSION,
        id,
        method: ETH_CALL,
        params: (
            RpcCall {
                to: contract.to_string(),
                data,
            },
            block,
        ),
    })
}

fn serialize_request<T: Serialize>(request: &T) -> Result<Vec<u8>, PmPolygonChainSourceError> {
    serde_json::to_vec(request).map_err(|_| PmPolygonChainSourceError::RequestEncoding)
}

fn address_pair_calldata(selector: &str, owner: EvmAddress, spender: EvmAddress) -> String {
    let owner = owner.to_string();
    let spender = spender.to_string();
    format!("0x{selector}{:0>64}{:0>64}", &owner[2..], &spender[2..])
}

pub(crate) fn decode_chain_id(body: &[u8]) -> Result<u64, PmPolygonChainSourceError> {
    let encoded: String = decode_response(body, CHAIN_ID_REQUEST_ID)?;
    parse_quantity_u64(&encoded)
}

pub(crate) fn decode_finalized_block(
    body: &[u8],
    expected_id: u64,
) -> Result<ParsedRpcBlock, PmPolygonChainSourceError> {
    let block: RpcBlockResult = decode_response(body, expected_id)?;
    let number = parse_quantity_u64(&block.number)?;
    let timestamp = parse_quantity_u64(&block.timestamp)?;
    let hash = parse_word(&block.hash, PmPolygonChainSourceError::InvalidBlockHash)?;
    if hash.iter().all(|byte| *byte == 0) {
        return Err(PmPolygonChainSourceError::ZeroBlockHash);
    }
    Ok(ParsedRpcBlock {
        identity: PmPolygonFinalizedBlock {
            number,
            hash,
            timestamp,
        },
        canonical_number: block.number,
    })
}

pub(crate) fn decode_allowance(body: &[u8]) -> Result<U256, PmPolygonChainSourceError> {
    let encoded: String = decode_response(body, PUSD_ALLOWANCE_REQUEST_ID)?;
    Ok(U256::from_be_bytes(parse_word(
        &encoded,
        PmPolygonChainSourceError::InvalidAllowanceWord,
    )?))
}

pub(crate) fn decode_approval(
    body: &[u8],
) -> Result<PmErc1155OperatorApproval, PmPolygonChainSourceError> {
    let encoded: String = decode_response(body, CONDITIONAL_TOKENS_APPROVAL_REQUEST_ID)?;
    let word = parse_word(&encoded, PmPolygonChainSourceError::InvalidApprovalWord)?;
    if word.iter().all(|byte| *byte == 0) {
        return Ok(PmErc1155OperatorApproval::from_bool(false));
    }
    if word[..31].iter().all(|byte| *byte == 0) && word[31] == 1 {
        return Ok(PmErc1155OperatorApproval::from_bool(true));
    }
    Err(PmPolygonChainSourceError::NonCanonicalApprovalBoolean)
}

fn decode_response<T: DeserializeOwned>(
    body: &[u8],
    expected_id: u64,
) -> Result<T, PmPolygonChainSourceError> {
    let response: RpcResponse =
        serde_json::from_slice(body).map_err(|_| PmPolygonChainSourceError::MalformedJsonRpc)?;
    if response.jsonrpc != JSON_RPC_VERSION {
        return Err(PmPolygonChainSourceError::WrongJsonRpcVersion);
    }
    if response.id != expected_id {
        return Err(PmPolygonChainSourceError::WrongResponseId {
            expected: expected_id,
            actual: response.id,
        });
    }
    match (response.result, response.error) {
        (Some(result), None) => {
            serde_json::from_value(result).map_err(|_| PmPolygonChainSourceError::MalformedJsonRpc)
        }
        (None, Some(error)) => {
            if error.message.is_empty()
                || error.message.len() > MAX_RPC_ERROR_MESSAGE_BYTES
                || error.message.bytes().any(|byte| byte.is_ascii_control())
            {
                return Err(PmPolygonChainSourceError::MalformedJsonRpcError);
            }
            let _bounded_data = error.data;
            Err(PmPolygonChainSourceError::RemoteRpcError { code: error.code })
        }
        _ => Err(PmPolygonChainSourceError::InvalidJsonRpcOutcome),
    }
}

fn parse_quantity_u64(encoded: &str) -> Result<u64, PmPolygonChainSourceError> {
    let digits = encoded
        .strip_prefix("0x")
        .ok_or(PmPolygonChainSourceError::NonCanonicalQuantity)?;
    if digits.is_empty()
        || digits.len() > 16
        || (digits.len() > 1 && digits.starts_with('0'))
        || digits
            .bytes()
            .any(|byte| !byte.is_ascii_hexdigit() || byte.is_ascii_uppercase())
    {
        return Err(PmPolygonChainSourceError::NonCanonicalQuantity);
    }
    u64::from_str_radix(digits, 16).map_err(|_| PmPolygonChainSourceError::NonCanonicalQuantity)
}

fn parse_word(
    encoded: &str,
    error: PmPolygonChainSourceError,
) -> Result<[u8; 32], PmPolygonChainSourceError> {
    let Some(digits) = encoded.strip_prefix("0x") else {
        return Err(error);
    };
    if digits.len() != 64
        || digits
            .bytes()
            .any(|byte| !byte.is_ascii_hexdigit() || byte.is_ascii_uppercase())
    {
        return Err(error);
    }
    let mut bytes = [0_u8; 32];
    for (index, output) in bytes.iter_mut().enumerate() {
        let high = hex_value(digits.as_bytes()[index * 2]).ok_or(error)?;
        let low = hex_value(digits.as_bytes()[index * 2 + 1]).ok_or(error)?;
        *output = (high << 4) | low;
    }
    Ok(bytes)
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}
