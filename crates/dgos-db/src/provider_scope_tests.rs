use crate::{migrations::Migrator, ModelRepository, ProviderRepository};
use dgos_types::{DgosError, Model, ProviderConfig};
use sea_orm::Database;
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
