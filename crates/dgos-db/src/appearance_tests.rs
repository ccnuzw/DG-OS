//! Full snapshot persistence, concurrency and recovery against real SQLite.
use crate::{migrations::Migrator, AppearanceRepository};
use dgos_types::{AppearanceRead, AppearanceSave, AppearanceValues};
use sea_orm::{ConnectionTrait, Database, DatabaseConnection, Statement};
use sea_orm_migration::MigratorTrait;
use serde_json::json;
async fn memory()->DatabaseConnection{let db=Database::connect("sqlite::memory:").await.unwrap();Migrator::up(&db,None).await.unwrap();db}
async fn seed(db:&DatabaseConnection,key:&str,value:&str){db.execute(Statement::from_sql_and_values(db.get_database_backend(),"INSERT OR REPLACE INTO system_settings(user_id,setting_key,setting_value,updated_at) VALUES ('default-user',?,?,0)",[key.into(),value.into()])).await.unwrap();}
fn snapshot(read:AppearanceRead)->dgos_types::AppearanceSnapshot{match read{AppearanceRead::Ready{snapshot,..}=>snapshot,_=>panic!("expected ready")}}
#[tokio::test]
async fn appearance_new_install_and_accounts_are_independent(){
 let repo=AppearanceRepository::new(memory().await);let fresh=snapshot(repo.get("default-user").await.unwrap());assert_eq!(fresh.values,AppearanceValues::default());
 let mut value=fresh.values.clone();value.mode="dark".into();repo.apply("default-user",&value,0).await.unwrap();assert_eq!(snapshot(repo.get("alice").await.unwrap()).values.mode,"light");
}
#[tokio::test]
async fn appearance_concurrent_cas_only_one_commit_and_conflict_returns_winner(){
 let repo=AppearanceRepository::new(memory().await);repo.get("default-user").await.unwrap();
 let mut a=AppearanceValues::default();a.mode="dark".into();let mut b=a.clone();b.density=175;
 let (a,b)=tokio::join!(repo.apply("default-user",&a,0),repo.apply("default-user",&b,0));
 let results=[a.unwrap(),b.unwrap()];assert_eq!(results.iter().filter(|r|matches!(r,AppearanceSave::Saved{..})).count(),1);
 let winner=snapshot(repo.get("default-user").await.unwrap());assert_eq!(winner.revision,1);
 for result in results{match result{AppearanceSave::Saved{snapshot}|AppearanceSave::Conflict{snapshot,..}=>assert_eq!(snapshot,winner)}}
}
#[tokio::test]
async fn appearance_storage_failure_leaves_complete_snapshot_unchanged(){
 let db=memory().await;let repo=AppearanceRepository::new(db.clone());let before=snapshot(repo.get("default-user").await.unwrap());
 db.execute_unprepared("CREATE TRIGGER fail_snapshot BEFORE UPDATE ON system_settings BEGIN SELECT RAISE(ABORT,'injected'); END").await.unwrap();
 let mut values=before.values.clone();values.mode="dark".into();values.density=175;assert!(repo.apply("default-user",&values,0).await.is_err());assert_eq!(snapshot(repo.get("default-user").await.unwrap()),before);
}
#[tokio::test]
async fn appearance_corruption_requires_owned_one_shot_token_and_preserves_raw_until_recovery(){
 let db=memory().await;seed(&db,"appearance.snapshot.v2","broken").await;let repo=AppearanceRepository::new(db.clone());
 let token=match repo.get("default-user").await.unwrap(){AppearanceRead::Corrupt{recovery_token,..}=>recovery_token,_=>panic!("expected corrupt")};
 assert!(repo.recover("alice",&token).await.is_err());assert!(repo.apply("default-user",&AppearanceValues::default(),0).await.is_err());
 let recovered=repo.recover("default-user",&token).await.unwrap();assert_eq!(recovered.revision,0);assert!(repo.recover("default-user",&token).await.is_err());
 seed(&db,"appearance.snapshot.v2","broken").await;let token=match repo.get("default-user").await.unwrap(){AppearanceRead::Corrupt{recovery_token,..}=>recovery_token,_=>panic!()};seed(&db,"appearance.snapshot.v2","changed").await;assert!(repo.recover("default-user",&token).await.is_err());
}
#[tokio::test]
async fn appearance_future_schema_is_never_overwritten(){
 let db=memory().await;seed(&db,"appearance.snapshot.v2",&json!({"schemaVersion":3,"revision":9}).to_string()).await;let repo=AppearanceRepository::new(db);assert!(matches!(repo.get("default-user").await.unwrap(),AppearanceRead::UnsupportedSchema{schema_version:3,..}));assert!(repo.apply("default-user",&AppearanceValues::default(),9).await.is_err());assert!(repo.recover("default-user","arbitrary").await.is_err());
}
#[tokio::test]
async fn appearance_valid_fields_survive_corrupt_leaves_without_rewriting_row(){
 let db=memory().await;let mut value=serde_json::to_value(AppearanceValues::default()).unwrap();value["revision"]=json!(7);value["mode"]=json!("dark");value["typography"]["codeSize"]=json!(999);value["density"]=json!(125);seed(&db,"appearance.snapshot.v2",&value.to_string()).await;
 let repo=AppearanceRepository::new(db.clone());match repo.get("default-user").await.unwrap(){AppearanceRead::Ready{snapshot,repair_warnings}=>{assert_eq!(snapshot.revision,7);assert_eq!(snapshot.values.mode,"dark");assert_eq!(snapshot.values.density,125);assert_eq!(snapshot.values.typography.code_size,12);assert!(!repair_warnings.is_empty());},_=>panic!()}
 let raw=db.query_one(Statement::from_string(db.get_database_backend(),"SELECT setting_value FROM system_settings WHERE user_id='default-user' AND setting_key='appearance.snapshot.v2'")).await.unwrap().unwrap().try_get::<String>("","setting_value").unwrap();
 assert_eq!(raw,value.to_string());
}

#[tokio::test]
async fn appearance_coupled_background_foreground_round_trip() {
 let repo=AppearanceRepository::new(memory().await);
 let mut values=AppearanceValues::default();
 values.variants.light.overrides.reading.background=Some("#111111".into());
 values.variants.light.overrides.reading.foreground=Some("#eeeeee".into());
 repo.apply("default-user",&values,0).await.unwrap();
 assert_eq!(snapshot(repo.get("default-user").await.unwrap()).values,values);
}

/// Recovery restores an offline database and its matching files, then runs supported forward
/// migrations. Restarting an older executable over the mutated database is not a rollback.
#[tokio::test]
async fn appearance_migration_failure_restores_database_and_files_before_forward_upgrade() {
 let root=tempfile::tempdir().unwrap();
 let live=root.path().join("live");let backup=root.path().join("backup");
 std::fs::create_dir(&live).unwrap();std::fs::create_dir(&backup).unwrap();
 let url=format!("sqlite://{}?mode=rwc",live.join("dgos.db").display());
 let db=crate::establish_connection(&url).await.unwrap();
 let before_generation=Migrator::migrations().iter().position(|m|m.name()=="m20261104_000022_generation_snapshots").unwrap();
 Migrator::up(&db,Some(before_generation as u32)).await.unwrap();
 seed(&db,"appearance.theme","dark").await;
 let repo=AppearanceRepository::new(db.clone());
 // D126 deliberately does not migrate the retired two-key preference.
 assert_eq!(snapshot(repo.get("default-user").await.unwrap()).values,AppearanceValues::default());
 let mut values=AppearanceValues::default();values.mode="dark".into();values.density=150;
 repo.apply("default-user",&values,0).await.unwrap();
 let expected=snapshot(repo.get("default-user").await.unwrap());
 db.execute_unprepared("INSERT INTO chat_sessions(id,user_id,title,skill_id,skill_version_id,skill_content_hash,created_at,updated_at) VALUES ('legacy','default-user','kept','skill','v1','hash',0,0)").await.unwrap();
 std::fs::write(live.join("owned-package"),b"matching supported files").unwrap();
 drop(repo);db.close().await.unwrap();
 // Every pool is closed before copying; no WAL/journal is omitted from a live database.
 for name in ["dgos.db","owned-package"] {std::fs::copy(live.join(name),backup.join(name)).unwrap();}
 let db=crate::establish_connection(&url).await.unwrap();
 db.execute_unprepared("CREATE TRIGGER fail_generation_registration BEFORE INSERT ON seaql_migrations WHEN NEW.version='m20261104_000022_generation_snapshots' BEGIN SELECT RAISE(ABORT,'injected migration registration failure'); END").await.unwrap();
 assert!(Migrator::up(&db,None).await.is_err());
 std::fs::write(live.join("owned-package"),b"changed after backup").unwrap();
 db.close().await.unwrap();
 // Restore the pair, rather than attempting down() on forward-only migrations.
 for name in ["dgos.db","owned-package"] {std::fs::copy(backup.join(name),live.join(name)).unwrap();}
 let db=crate::establish_connection(&url).await.unwrap();
 let columns=db.query_all(Statement::from_string(db.get_database_backend(),"PRAGMA table_info(chat_sessions)")).await.unwrap();
 assert!(!columns.iter().any(|r|r.try_get::<String>("","name").unwrap()=="skill_refs"));
 assert_eq!(snapshot(AppearanceRepository::new(db.clone()).get("default-user").await.unwrap()),expected);
 assert_eq!(std::fs::read(live.join("owned-package")).unwrap(),b"matching supported files");
 Migrator::up(&db,None).await.unwrap();db.close().await.unwrap();
 let reopened=crate::establish_connection(&url).await.unwrap();
 let repo=AppearanceRepository::new(reopened.clone());
 assert_eq!(snapshot(repo.get("default-user").await.unwrap()),expected);
 assert_eq!(snapshot(repo.get("alice").await.unwrap()).values,AppearanceValues::default());
 let row=reopened.query_one(Statement::from_string(reopened.get_database_backend(),"SELECT skill_refs,generation_parameters FROM chat_sessions WHERE id='legacy'")).await.unwrap().unwrap();
 let refs:serde_json::Value=serde_json::from_str(&row.try_get::<String>("","skill_refs").unwrap()).unwrap();
 assert_eq!(refs,json!([{"skill_id":"skill","version_id":"v1","content_hash":"hash"}]));
 assert_eq!(row.try_get::<String>("","generation_parameters").unwrap(),"{}");
 drop(repo);reopened.close().await.unwrap();
}

#[tokio::test]
async fn appearance_session_end_and_future_row_invalidate_corruption_recovery() {
 let db=memory().await;seed(&db,"appearance.snapshot.v2","broken").await;
 let repo=AppearanceRepository::new(db.clone());
 let token=match repo.get("default-user").await.unwrap(){AppearanceRead::Corrupt{recovery_token,..}=>recovery_token,_=>panic!()};
 repo.invalidate_recovery("default-user");
 assert!(repo.recover("default-user",&token).await.is_err());
 let token=match repo.get("default-user").await.unwrap(){AppearanceRead::Corrupt{recovery_token,..}=>recovery_token,_=>panic!()};
 let future=json!({"schemaVersion":3,"revision":9}).to_string();
 seed(&db,"appearance.snapshot.v2",&future).await;
 assert!(repo.recover("default-user",&token).await.is_err());
 let raw=db.query_one(Statement::from_string(db.get_database_backend(),"SELECT setting_value FROM system_settings WHERE user_id='default-user' AND setting_key='appearance.snapshot.v2'")).await.unwrap().unwrap().try_get::<String>("","setting_value").unwrap();
 assert_eq!(raw,future);
}

#[tokio::test]
async fn appearance_failed_recovery_preserves_raw_and_requires_a_new_token() {
 let db=memory().await;seed(&db,"appearance.snapshot.v2","broken").await;
 let repo=AppearanceRepository::new(db.clone());
 let token=match repo.get("default-user").await.unwrap(){AppearanceRead::Corrupt{recovery_token,..}=>recovery_token,_=>panic!()};
 db.execute_unprepared("CREATE TRIGGER fail_recovery BEFORE UPDATE ON system_settings BEGIN SELECT RAISE(ABORT,'injected'); END").await.unwrap();
 assert!(repo.recover("default-user",&token).await.is_err());
 let raw=db.query_one(Statement::from_string(db.get_database_backend(),"SELECT setting_value FROM system_settings WHERE user_id='default-user' AND setting_key='appearance.snapshot.v2'")).await.unwrap().unwrap().try_get::<String>("","setting_value").unwrap();
 assert_eq!(raw,"broken");
 db.execute_unprepared("DROP TRIGGER fail_recovery").await.unwrap();
 assert!(repo.recover("default-user",&token).await.is_err());
 let fresh=match repo.get("default-user").await.unwrap(){AppearanceRead::Corrupt{recovery_token,..}=>recovery_token,_=>panic!()};
 let recovered=repo.recover("default-user",&fresh).await.unwrap();
 assert_eq!(recovered.revision,0);
 assert_eq!(recovered.values,AppearanceValues::default());
 assert_eq!(snapshot(repo.get("default-user").await.unwrap()),recovered);
}
