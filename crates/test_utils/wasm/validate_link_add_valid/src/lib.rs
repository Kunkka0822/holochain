use hdk3::prelude::*;

#[hdk_extern]
pub fn validate_link(_: ValidateLinkAddData) -> ExternResult<ValidateLinkAddCallbackResult> {
    Ok(ValidateLinkAddCallbackResult::Valid)
}
