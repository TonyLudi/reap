use super::*;

pub(super) fn validate_header(header: &PmCaptureHeader) -> Result<(), PmCaptureVerifyError> {
    validate_provenance(&header.provenance)?;
    if header.schema_version != PM_PUBLIC_CAPTURE_SCHEMA_VERSION
        || header.product != PM_PUBLIC_CAPTURE_PRODUCT
        || !valid_digest(&header.configuration_sha256)
        || !valid_digest(&header.structural_scope_sha256)
        || header.authenticated
        || header.production_order_entry_authorized
    {
        return Err(PmCaptureVerifyError::InvalidHeader(
            "schema, identity, units, scope, or authority is invalid",
        ));
    }
    let scope_bytes =
        serde_json::to_vec(&header.scope).map_err(PmCaptureVerifyError::SerializeHeader)?;
    if header.structural_scope_sha256 != sha256_hex(&scope_bytes) {
        return Err(PmCaptureVerifyError::InvalidHeader(
            "structural scope fingerprint mismatched",
        ));
    }
    let configuration_bytes = serde_json::to_vec(&(&header.scope, header.session_policy))
        .map_err(PmCaptureVerifyError::SerializeHeader)?;
    if header.configuration_sha256 != sha256_hex(&configuration_bytes) {
        return Err(PmCaptureVerifyError::InvalidHeader(
            "configuration fingerprint mismatched",
        ));
    }
    header.session_policy.validate()?;
    validate_scope(&header.scope)
}

pub(super) fn validate_provenance(value: &PmCaptureProvenance) -> Result<(), PmCaptureVerifyError> {
    let fields = [
        value.reference_commit.as_str(),
        value.reference_blob_oid.as_str(),
        value.reference_seed_sha256.as_str(),
        value.fixture_sha256.as_str(),
    ];
    if fields.iter().any(|field| {
        field.is_empty()
            || field.len() > MAX_PROVENANCE_TEXT_BYTES
            || !field.bytes().all(|byte| byte.is_ascii_hexdigit())
    }) || value.reference_blob_oid.len() != 40
        || !valid_digest(&value.reference_seed_sha256)
        || !valid_digest(&value.fixture_sha256)
        || value.reference_commit != PREDARB_REFERENCE_COMMIT
        || value.reference_blob_oid != "bbb5bc143a914ba8c96d84342321b3dba30ec0fc"
        || value.reference_seed_sha256 != PREDARB_REFERENCE_SEED_SHA256
    {
        Err(PmCaptureVerifyError::InvalidProvenance)
    } else {
        Ok(())
    }
}

pub(super) fn validate_scope(scope: &PmCaptureScope) -> Result<(), PmCaptureVerifyError> {
    let okx_route_matches = matches!(
        scope.okx_source,
        PmProductSource::OkxReference { reference, .. }
            if reference == scope.okx_reference
    );
    if !okx_route_matches
        || scope.instrument.token()
            != match scope.source {
                PmProductSource::PolymarketMarket { token, .. } => token,
                _ => {
                    return Err(PmCaptureVerifyError::InvalidHeader(
                        "source is not PM market",
                    ));
                }
            }
        || scope.metadata.condition() != scope.condition
        || scope.metadata.market() != scope.market
        || scope.metadata.outcome().token() != scope.outcome_token
        || scope.raw_pm_instrument
            != PmInstrumentId::new(scope.metadata.market(), scope.metadata.outcome().token())
        || scope.metadata.tick() != scope.tick
        || scope.metadata.minimum_order_size() != scope.minimum_order_size
        || scope.metadata.negative_risk() != scope.negative_risk
        || scope.metadata_revision.value() == 0
        || scope.metadata_monotonic_receive_ns == 0
        || !valid_digest(&scope.metadata_sha256)
        || !valid_digest(&scope.domain_sha256)
        || !valid_digest(&scope.identity_configuration_sha256)
        || scope.tick.units() == 0
        || scope.minimum_order_size.protocol_units().is_zero()
        || scope.price_units_per_one != PM_PROTOCOL_SCALE
        || scope.quantity_units_per_one != PM_PROTOCOL_SCALE
        || scope.collateral_units_per_one != PM_PROTOCOL_SCALE
        || scope.lot_units != CLOB_V2_LOT_UNITS
    {
        return Err(PmCaptureVerifyError::InvalidHeader(
            "metadata contract, scope, or integral units drifted",
        ));
    }
    PmPrice::from_units(scope.tick.units())
        .map_err(|_| PmCaptureVerifyError::InvalidHeader("tick is outside (0,1)"))?;
    scope.authoritative_metadata()?;
    scope.observation_grant()?;
    Ok(())
}
