use crate::{ProviderManager, test_support::{MemoryCredentialVault, seed_provider}};
use dgos_db::{migrations::Migrator, sea_orm_migration::MigratorTrait, ModelRepository};
use dgos_types::{Model, ProviderConfig, DgosError};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn transcription_requires_owner_and_model_and_never_retries_unknown_results() {
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}?mode=rwc", dir.path().join("voice.db").display());
    let db = dgos_db::establish_connection(&url).await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    let vault = Arc::new(MemoryCredentialVault::default());
    let mut server = mockito::Server::new_async().await;
    let provider = seed_provider(db.clone(), &vault, ProviderConfig { name: "Controlled voice".into(), provider_type: "openai_compatible".into(), base_url: server.url(), api_key: "synthetic-voice".into(), enabled: true, config: None }, "alice").await.unwrap();
    ModelRepository::new(db).sync_models(&provider.id, vec![Model { model_id: "whisper-1".into(), model_name: "Transcription".into(), model_type: "speech-transcription".into(), ..Default::default() }]).await.unwrap();
    let manager = ProviderManager::with_vault(&url, vault).await.unwrap();
    let audio = b"OggScontrolled-recording".to_vec();
    let call = || CancellationToken::new();
    assert!(matches!(manager.transcribe_audio("bob", &provider.id, "whisper-1", "audio/ogg", audio.clone(), call()).await, Err(DgosError::ProviderNotFound(_))));
    assert!(matches!(manager.transcribe_audio("alice", &provider.id, "chat-model", "audio/ogg", audio.clone(), call()).await, Err(DgosError::RequestNotSupported)));
    let success = server.mock("POST", "/audio/transcriptions").match_header("authorization", "Bearer synthetic-voice").match_body(mockito::Matcher::Regex("name=\"model\"\\r\\n\\r\\nwhisper-1".into())).match_body(mockito::Matcher::Regex("OggScontrolled-recording".into())).with_status(200).with_body(r#"{"text":"editable result"}"#).expect(1).create_async().await;
    assert_eq!(manager.transcribe_audio("alice", &provider.id, "whisper-1", "audio/ogg", audio.clone(), call()).await.unwrap(), "editable result");
    success.assert_async().await;
    success.remove_async().await;
    let failed = server.mock("POST", "/audio/transcriptions").with_status(503).with_body("sensitive upstream text").expect(1).create_async().await;
    assert!(matches!(manager.transcribe_audio("alice", &provider.id, "whisper-1", "audio/ogg", audio.clone(), call()).await, Err(DgosError::UpstreamServerError)));
    failed.assert_async().await;
    let cancelled = call(); cancelled.cancel();
    assert!(matches!(manager.transcribe_audio("alice", &provider.id, "whisper-1", "audio/ogg", audio, cancelled).await, Err(DgosError::StreamInterrupted)));
    failed.assert_async().await;
}
