use crate::{migrations::Migrator, ModelRepository, ProviderRepository};
use dgos_types::{
    build_model, recompute_models, DgosError, Hint, KindSource, Model, ModelKind, ModelSource,
    ProviderConfig,
};
use sea_orm::{ConnectionTrait, Database, DbBackend, Statement};
use sea_orm_migration::MigratorTrait;

fn config(name: &str) -> ProviderConfig {
    ProviderConfig {
        name: name.into(),
        provider_type: "openai_compatible".into(),
        base_url: "https://example.invalid/v1".into(),
        api_key: String::new(),
        enabled: true,
        config: None,
    }
}

fn model(id: &str) -> Model {
    Model {
        id: id.into(),
        provider_id: String::new(),
        model_id: id.into(),
        model_name: id.into(),
        model_type: "chat".into(),
        context_window: Some(8192),
        max_output_tokens: None,
        supports_streaming: true,
        supports_vision: false,
        cached_at: String::new(),
        ..Default::default()
    }
}

async fn database() -> sea_orm::DatabaseConnection {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    db
}

#[tokio::test]
async fn provider_and_initial_models_commit_together() {
    let db = database().await;
    let repo = ProviderRepository::new(db.clone());
    let created = repo
        .create_with_models(
            "fixture".into(),
            config("Initial"),
            "local".into(),
            Some("synthetic-reference".into()),
            vec![model("first"), model("second")],
        )
        .await
        .unwrap();
    assert_eq!(created.config_revision, 0);
    let (snapshot, cached) = repo
        .model_snapshot(&created.id, "local", "first")
        .await
        .unwrap();
    assert_eq!(
        snapshot.credential_ref.as_deref(),
        Some("synthetic-reference")
    );
    assert_eq!(cached.provider_id, created.id);
    assert_eq!(
        ModelRepository::new(db)
            .list(&created.id)
            .await
            .unwrap()
            .len(),
        2
    );
}

#[tokio::test]
async fn duplicate_initial_model_ids_leave_no_partial_provider() {
    let db = database().await;
    let repo = ProviderRepository::new(db.clone());
    let result = repo
        .create_with_models(
            "fixture".into(),
            config("Initial"),
            "local".into(),
            None,
            vec![model("duplicate"), model("duplicate")],
        )
        .await;
    assert!(matches!(result, Err(DgosError::DatabaseError(_))));
    assert!(repo.list("local").await.unwrap().is_empty());
    assert!(ModelRepository::new(db)
        .list("fixture")
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn latest_started_refresh_publishes_one_coherent_snapshot() {
    let db = database().await;
    let repo = ProviderRepository::new(db.clone());
    let provider = repo
        .create_with_models(
            "fixture".into(),
            config("Initial"),
            "local".into(),
            None,
            vec![model("initial")],
        )
        .await
        .unwrap();
    let earlier = repo
        .begin_model_refresh(&provider.id, "local")
        .await
        .unwrap();
    let latest = repo
        .begin_model_refresh(&provider.id, "local")
        .await
        .unwrap();
    assert!(matches!(
        repo.publish_model_refresh(&earlier, vec![model("earlier")])
            .await,
        Err(DgosError::ProviderConfigurationChanged)
    ));
    let published = repo
        .publish_model_refresh(&latest, vec![model("latest")])
        .await
        .unwrap();
    assert_eq!(published[0].model_id, "latest");
    let (snapshot, cached) = repo
        .model_snapshot(&provider.id, "local", "latest")
        .await
        .unwrap();
    assert_eq!(snapshot.config_revision, provider.config_revision + 1);
    assert_eq!(cached.provider_id, provider.id);
    assert!(matches!(
        repo.publish_model_refresh(&latest, vec![model("repeat")])
            .await,
        Err(DgosError::ProviderConfigurationChanged)
    ));
    assert_eq!(
        ModelRepository::new(db).list(&provider.id).await.unwrap()[0].model_id,
        "latest"
    );
}

#[tokio::test]
async fn metadata_preserves_cache_and_invalidates_refresh_while_rotation_clears_cache() {
    let db = database().await;
    let repo = ProviderRepository::new(db.clone());
    let cache = ModelRepository::new(db);
    let provider = repo
        .create_with_models(
            "fixture".into(),
            config("Initial"),
            "local".into(),
            Some("initial-ref".into()),
            vec![model("initial")],
        )
        .await
        .unwrap();
    let ticket = repo
        .begin_model_refresh(&provider.id, "local")
        .await
        .unwrap();
    let renamed = repo
        .update(&provider.id, config("Renamed"), "local")
        .await
        .unwrap();
    assert_eq!(renamed.config_revision, provider.config_revision + 1);
    assert_eq!(
        cache.list(&provider.id).await.unwrap()[0].model_id,
        "initial"
    );
    assert!(matches!(
        repo.publish_model_refresh(&ticket, vec![model("old-refresh")])
            .await,
        Err(DgosError::ProviderConfigurationChanged)
    ));
    let ticket = repo
        .begin_model_refresh(&provider.id, "local")
        .await
        .unwrap();
    let rotated = repo
        .update_with_credential_ref(
            &provider.id,
            config("Rotated"),
            "local",
            Some("rotated-ref".into()),
        )
        .await
        .unwrap();
    assert_eq!(rotated.config_revision, renamed.config_revision + 1);
    assert!(cache.list(&provider.id).await.unwrap().is_empty());
    assert!(matches!(
        repo.publish_model_refresh(&ticket, vec![model("old-refresh")])
            .await,
        Err(DgosError::ProviderConfigurationChanged)
    ));
}

#[tokio::test]
async fn unsuccessful_cache_replacement_preserves_rows_and_revision() {
    let db = database().await;
    let repo = ProviderRepository::new(db.clone());
    let cache = ModelRepository::new(db);
    let provider = repo
        .create_with_models(
            "fixture".into(),
            config("Initial"),
            "local".into(),
            None,
            vec![model("initial")],
        )
        .await
        .unwrap();
    let ticket = repo
        .begin_model_refresh(&provider.id, "local")
        .await
        .unwrap();
    assert!(matches!(
        repo.publish_model_refresh(&ticket, vec![model("duplicate"), model("duplicate")])
            .await,
        Err(DgosError::DatabaseError(_))
    ));
    assert_eq!(
        repo.get(&provider.id, "local")
            .await
            .unwrap()
            .unwrap()
            .config_revision,
        provider.config_revision
    );
    assert_eq!(
        cache.list(&provider.id).await.unwrap()[0].model_id,
        "initial"
    );
    assert!(cache
        .sync_models(&provider.id, vec![model("duplicate"), model("duplicate")])
        .await
        .is_err());
    assert_eq!(
        cache.list(&provider.id).await.unwrap()[0].model_id,
        "initial"
    );
    repo.publish_model_refresh(&ticket, vec![model("retried")])
        .await
        .unwrap();
    assert_eq!(
        cache.list(&provider.id).await.unwrap()[0].model_id,
        "retried"
    );
}

#[tokio::test]
async fn scoped_provider_crud_and_transactional_order_preserve_normal_lifecycle() {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    let repo = ProviderRepository::new(db);
    let first = repo
        .create(config("First"), "local-user".into())
        .await
        .unwrap();
    let second = repo
        .create(config("Second"), "local-user".into())
        .await
        .unwrap();
    assert_eq!(
        repo.get(&first.id, "local-user")
            .await
            .unwrap()
            .unwrap()
            .user_id,
        "local-user"
    );

    repo.update_order(vec![second.id.clone(), first.id.clone()], "local-user")
        .await
        .unwrap();
    let ordered = repo.list("local-user").await.unwrap();
    assert_eq!(
        ordered.iter().map(|row| row.sort_order).collect::<Vec<_>>(),
        vec![0, 1]
    );
    assert_eq!(ordered[0].id, second.id);
    let updated = repo
        .update(&first.id, config("Renamed"), "local-user")
        .await
        .unwrap();
    assert_eq!(updated.name, "Renamed");
    assert!(
        !repo
            .toggle_enabled(&first.id, false, "local-user")
            .await
            .unwrap()
            .enabled
    );
    repo.delete(&second.id, "local-user").await.unwrap();
    assert_eq!(repo.list("local-user").await.unwrap().len(), 1);
}

fn host_model(model_id: &str) -> Model {
    build_model(
        "openai_compatible",
        model_id,
        None,
        ModelSource::Remote,
        None,
    )
}

#[tokio::test]
async fn model_kind_roundtrip() {
    let db = database().await;
    let repo = ProviderRepository::new(db.clone());
    let provider = repo
        .create_with_models(
            "fixture".into(),
            config("Kinds"),
            "local".into(),
            None,
            ["gpt-4o", "text-embedding-3-small", "dall-e-3", "whisper-1"]
                .into_iter()
                .map(host_model)
                .collect(),
        )
        .await
        .unwrap();
    let listed = ModelRepository::new(db.clone())
        .list(&provider.id)
        .await
        .unwrap();
    let by_id = |id: &str| listed.iter().find(|m| m.model_id == id).unwrap();
    assert_eq!(by_id("gpt-4o").model_type, "text-generation");
    assert_eq!(by_id("gpt-4o").kind_source, KindSource::Fallback);
    assert!(by_id("gpt-4o").supports_streaming);
    assert_eq!(by_id("text-embedding-3-small").model_type, "embedding");
    assert_eq!(
        by_id("text-embedding-3-small").kind_source,
        KindSource::Rule
    );
    assert!(!by_id("text-embedding-3-small").supports_streaming);
    assert_eq!(by_id("dall-e-3").model_type, "image-generation");
    assert_eq!(by_id("whisper-1").model_type, "speech-transcription");
    assert!(listed.iter().all(|m| m.source == ModelSource::Remote));

    // max_output_tokens is persisted when the host supplies it and stays empty otherwise.
    let mut with_limit = host_model("limited");
    with_limit.max_output_tokens = Some(4096);
    with_limit.context_window = Some(128_000);
    repo.add_manual_model(&provider.id, "local", with_limit)
        .await
        .unwrap();
    let limited = ModelRepository::new(db)
        .get(&provider.id, "limited")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(limited.max_output_tokens, Some(4096));
    assert_eq!(limited.context_window, Some(128_000));
    assert_eq!(by_id("gpt-4o").max_output_tokens, None);
}

#[tokio::test]
async fn model_kind_roundtrip_derives_legacy_rows_without_writing_back() {
    let db = database().await;
    let repo = ProviderRepository::new(db.clone());
    let provider = repo
        .create_with_models(
            "fixture".into(),
            config("Legacy"),
            "local".into(),
            None,
            vec![],
        )
        .await
        .unwrap();
    // Rows as the pre-migration code wrote them: array capabilities, no kind columns.
    db.execute_unprepared(
        "INSERT INTO models (id, provider_id, model_id, model_name, capabilities, is_available, synced_at) VALUES \
         ('legacy-1', 'fixture', 'text-embedding-3-small', 'text-embedding-3-small', '[\"streaming\"]', 1, '2026-10-27T00:00:00+00:00'), \
         ('legacy-2', 'fixture', 'gpt-4-vision-preview', 'gpt-4-vision-preview', '[\"streaming\",\"vision\"]', 1, '2026-10-27T00:00:00+00:00'), \
         ('legacy-3', 'fixture', 'plain-model', 'plain-model', '[]', 1, '2026-10-27T00:00:00+00:00')",
    )
    .await
    .unwrap();
    let listed = ModelRepository::new(db.clone())
        .list(&provider.id)
        .await
        .unwrap();
    let by_id = |id: &str| listed.iter().find(|m| m.model_id == id).unwrap();
    assert_eq!(by_id("text-embedding-3-small").model_type, "embedding");
    assert_eq!(
        by_id("text-embedding-3-small").kind_source,
        KindSource::Rule
    );
    assert_eq!(by_id("gpt-4-vision-preview").model_type, "text-generation");
    assert!(by_id("gpt-4-vision-preview").supports_vision);
    assert_eq!(by_id("plain-model").model_type, "text-generation");
    assert_eq!(by_id("plain-model").capabilities.streaming, Hint::Unknown);
    assert!(listed.iter().all(|m| m.source == ModelSource::Remote));

    let untouched = db
        .query_one(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT COUNT(*) AS n FROM models WHERE kind IS NULL AND kind_source IS NULL",
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(untouched.try_get::<i64>("", "n").unwrap(), 3);
}

#[tokio::test]
async fn model_kind_roundtrip_reads_anthropic_rows_as_protocol_text_generation() {
    let db = database().await;
    let repo = ProviderRepository::new(db.clone());
    let mut anthropic = config("Anthropic");
    anthropic.provider_type = "anthropic".into();
    let provider = repo
        .create_with_models("fixture".into(), anthropic, "local".into(), None, vec![])
        .await
        .unwrap();
    db.execute_unprepared(
        "INSERT INTO models (id, provider_id, model_id, model_name, capabilities, is_available, synced_at) VALUES \
         ('legacy-1', 'fixture', 'text-embedding-like-name', 'x', '[]', 1, '2026-10-27T00:00:00+00:00')",
    )
    .await
    .unwrap();
    let listed = ModelRepository::new(db).list(&provider.id).await.unwrap();
    assert_eq!(listed[0].model_type, ModelKind::TextGeneration.as_str());
    assert_eq!(listed[0].kind_source, KindSource::Protocol);
}

#[tokio::test]
async fn manual_models_are_idempotent_and_keep_the_chosen_kind() {
    let db = database().await;
    let repo = ProviderRepository::new(db.clone());
    let provider = repo
        .create_with_models(
            "fixture".into(),
            config("Manual"),
            "local".into(),
            None,
            vec![],
        )
        .await
        .unwrap();
    let first = repo
        .add_manual_model(
            &provider.id,
            "local",
            build_model(
                "openai_compatible",
                "local-embedder",
                None,
                ModelSource::Manual,
                Some(ModelKind::Embedding),
            ),
        )
        .await
        .unwrap();
    assert_eq!(first.source, ModelSource::Manual);
    assert_eq!(first.model_type, "embedding");
    assert_eq!(first.kind_source, KindSource::Manual);
    let revision = repo
        .get(&provider.id, "local")
        .await
        .unwrap()
        .unwrap()
        .config_revision;
    assert_eq!(revision, provider.config_revision + 1);

    // The same id again returns the stored row and changes nothing.
    let again = repo
        .add_manual_model(
            &provider.id,
            "local",
            build_model(
                "openai_compatible",
                "local-embedder",
                None,
                ModelSource::Manual,
                Some(ModelKind::Rerank),
            ),
        )
        .await
        .unwrap();
    assert_eq!(again.id, first.id);
    assert_eq!(again.model_type, "embedding");
    assert_eq!(
        repo.get(&provider.id, "local")
            .await
            .unwrap()
            .unwrap()
            .config_revision,
        revision
    );
    let second = repo
        .add_manual_model(&provider.id, "local", host_model("llama3"))
        .await
        .unwrap();
    assert_eq!(second.source, ModelSource::Manual);
    assert_eq!(second.kind_source, KindSource::Fallback);
    assert_eq!(
        ModelRepository::new(db)
            .list(&provider.id)
            .await
            .unwrap()
            .len(),
        2
    );
}

#[tokio::test]
async fn manual_models_survive_refresh_and_skip_same_id_remote_items() {
    let db = database().await;
    let repo = ProviderRepository::new(db.clone());
    let provider = repo
        .create_with_models(
            "fixture".into(),
            config("Refresh"),
            "local".into(),
            None,
            vec![host_model("old-remote")],
        )
        .await
        .unwrap();
    repo.add_manual_model(&provider.id, "local", host_model("mine"))
        .await
        .unwrap();
    let ticket = repo
        .begin_model_refresh(&provider.id, "local")
        .await
        .unwrap();
    let published = repo
        .publish_model_refresh(&ticket, vec![host_model("new-remote"), host_model("mine")])
        .await
        .unwrap();
    let ids: std::collections::BTreeMap<_, _> = published
        .iter()
        .map(|m| (m.model_id.clone(), m.source))
        .collect();
    assert_eq!(ids.len(), 2, "{ids:?}");
    assert_eq!(ids["mine"], ModelSource::Manual);
    assert_eq!(ids["new-remote"], ModelSource::Remote);
    assert!(!ids.contains_key("old-remote"));
}

#[tokio::test]
async fn manual_models_removal_touches_only_manual_rows_and_advances_the_revision() {
    let db = database().await;
    let repo = ProviderRepository::new(db.clone());
    let provider = repo
        .create_with_models(
            "fixture".into(),
            config("Remove"),
            "local".into(),
            None,
            vec![host_model("remote-one")],
        )
        .await
        .unwrap();
    repo.add_manual_model(&provider.id, "local", host_model("mine"))
        .await
        .unwrap();
    let before = repo
        .get(&provider.id, "local")
        .await
        .unwrap()
        .unwrap()
        .config_revision;

    // A remote row and a missing id are successful no-ops.
    repo.remove_manual_model(&provider.id, "local", "remote-one")
        .await
        .unwrap();
    repo.remove_manual_model(&provider.id, "local", "absent")
        .await
        .unwrap();
    assert_eq!(
        repo.get(&provider.id, "local")
            .await
            .unwrap()
            .unwrap()
            .config_revision,
        before
    );
    assert_eq!(
        ModelRepository::new(db.clone())
            .list(&provider.id)
            .await
            .unwrap()
            .len(),
        2
    );

    let ticket = repo
        .begin_model_refresh(&provider.id, "local")
        .await
        .unwrap();
    repo.remove_manual_model(&provider.id, "local", "mine")
        .await
        .unwrap();
    assert_eq!(
        repo.get(&provider.id, "local")
            .await
            .unwrap()
            .unwrap()
            .config_revision,
        before + 1
    );
    // The removal moved the revision, so the refresh that started earlier is rejected.
    assert!(matches!(
        repo.publish_model_refresh(&ticket, vec![]).await,
        Err(DgosError::ProviderConfigurationChanged)
    ));
    let remaining = ModelRepository::new(db).list(&provider.id).await.unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].model_id, "remote-one");
}

#[tokio::test]
async fn manual_models_are_scoped_to_the_owning_user() {
    let db = database().await;
    let repo = ProviderRepository::new(db);
    let provider = repo
        .create_with_models(
            "fixture".into(),
            config("Owned"),
            "local".into(),
            None,
            vec![],
        )
        .await
        .unwrap();
    assert!(matches!(
        repo.add_manual_model(&provider.id, "someone-else", host_model("x"))
            .await,
        Err(DgosError::ProviderNotFound(_))
    ));
    assert!(matches!(
        repo.remove_manual_model(&provider.id, "someone-else", "x")
            .await,
        Err(DgosError::ProviderNotFound(_))
    ));
}

#[tokio::test]
async fn renderer_models_recomputed() {
    // What a compromised renderer might submit: forged key, kind, capabilities and lengths.
    let forged = Model {
        id: "forged-primary-key".into(),
        model_id: "text-embedding-3-small".into(),
        model_name: "Display name".into(),
        model_type: "text-generation".into(),
        context_window: Some(999_999),
        max_output_tokens: Some(999_999),
        supports_streaming: true,
        supports_vision: true,
        kind_source: KindSource::Manual,
        source: ModelSource::Manual,
        capabilities: dgos_types::ModelCapabilities {
            tools: Hint::Supported,
            ..Default::default()
        },
        ..Default::default()
    };
    let recomputed =
        recompute_models("openai_compatible", &[forged.clone(), forged.clone()]).unwrap();
    assert_eq!(recomputed.len(), 1, "duplicate ids collapse to one row");
    let model = &recomputed[0];
    assert_ne!(model.id, "forged-primary-key");
    assert_eq!(model.model_name, "Display name");
    assert_eq!(model.model_type, "embedding");
    assert_eq!(model.kind_source, KindSource::Rule);
    assert_eq!(model.source, ModelSource::Remote);
    assert_eq!(model.context_window, None);
    assert_eq!(model.max_output_tokens, None);
    assert!(!model.supports_streaming && !model.supports_vision);
    assert_eq!(model.capabilities.tools, Hint::Unknown);

    // The recomputed row is what reaches storage and comes back.
    let db = database().await;
    let repo = ProviderRepository::new(db.clone());
    let provider = repo
        .create_with_models(
            "fixture".into(),
            config("Recomputed"),
            "local".into(),
            None,
            recomputed,
        )
        .await
        .unwrap();
    let stored = ModelRepository::new(db).list(&provider.id).await.unwrap();
    assert_eq!(stored[0].model_type, "embedding");
    assert_eq!(stored[0].context_window, None);

    // One invalid id rejects the whole submission.
    for bad in ["", "   ", &"x".repeat(256)] {
        let mut invalid = forged.clone();
        invalid.model_id = bad.to_string();
        assert!(matches!(
            recompute_models("openai_compatible", &[forged.clone(), invalid]),
            Err(DgosError::RequestNotSupported)
        ));
    }
}
