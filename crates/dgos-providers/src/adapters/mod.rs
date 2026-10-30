pub mod anthropic;
pub mod openai;

use dgos_types::{ChatRequest, DgosError, DgosResult};

use crate::structured;
use crate::vendor::Profile;

/// Local refusals that need no upstream round trip (B01-46): a streaming call with a structured
/// output request, a sampling parameter the profile declares unusable, an unusable schema, or a
/// reasoning level the profile has no mapping for. Protocol-level impossibilities of one
/// protocol (Anthropic message order) are checked by that adapter.
pub(crate) fn preflight(profile: &Profile, request: &ChatRequest, stream: bool) -> DgosResult<()> {
    if stream && request.response_format.is_some() {
        return Err(DgosError::RequestNotSupported);
    }
    profile.check_sampling(request.temperature, request.reasoning.as_ref())?;
    if let Some(format) = &request.response_format {
        structured::check_format(format)?;
    }
    if let Some(reasoning) = &request.reasoning {
        profile.reasoning_params(reasoning)?;
    }
    Ok(())
}
