//! External native callers must be able to name the typed transient boundary. An internal
//! actor-module test alone would compile even if its re-export stopped at a private module.
use catcoms_app::{AppCommand, VoiceEndpointCommand, VoiceEndpointError};
use catcoms_sync::voice_v2::VoiceError;

#[test]
fn native_voice_api_exposes_typed_capacity_and_opaque_command_envelope() {
    let temporary = VoiceEndpointError::Protocol(VoiceError::Capacity);
    assert!(matches!(
        temporary,
        VoiceEndpointError::Protocol(VoiceError::Capacity)
    ));
    assert!(!matches!(
        VoiceEndpointError::Stopped,
        VoiceEndpointError::Protocol(_)
    ));
    // Type-level compatibility only: external code can name the envelope but cannot construct
    // its private command variants or serialize native authority into it.
    let envelope = None::<VoiceEndpointCommand>.map(AppCommand::VoiceEndpoint);
    assert!(envelope.is_none());
}
