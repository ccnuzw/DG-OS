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
 let a=AppearanceValues{mode:"dark".into(),..Default::default()};let mut b=a.clone();b.density=120;
 let (a,b)=tokio::join!(repo.apply("default-user",&a,0),repo.apply("default-user",&b,0));
 let results=[a.unwrap(),b.unwrap()];assert_eq!(results.iter().filter(|r|matches!(r,AppearanceSave::Saved{..})).count(),1);
 let winner=snapshot(repo.get("default-user").await.unwrap());assert_eq!(winner.revision,1);
 for result in results{match result{AppearanceSave::Saved{snapshot}|AppearanceSave::Conflict{snapshot,..}=>assert_eq!(snapshot,winner)}}
}
#[tokio::test]
async fn appearance_storage_failure_leaves_complete_snapshot_unchanged(){
 let db=memory().await;let repo=AppearanceRepository::new(db.clone());let before=snapshot(repo.get("default-user").await.unwrap());
 db.execute_unprepared("CREATE TRIGGER fail_snapshot BEFORE UPDATE ON system_settings BEGIN SELECT RAISE(ABORT,'injected'); END").await.unwrap();
 let mut values=before.values.clone();values.mode="dark".into();values.density=120;assert!(repo.apply("default-user",&values,0).await.is_err());assert_eq!(snapshot(repo.get("default-user").await.unwrap()),before);
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
 let db=memory().await;seed(&db,"appearance.snapshot.v2",&json!({"schemaVersion":7,"revision":9}).to_string()).await;let repo=AppearanceRepository::new(db);assert!(matches!(repo.get("default-user").await.unwrap(),AppearanceRead::UnsupportedSchema{schema_version:7,..}));assert!(repo.apply("default-user",&AppearanceValues::default(),9).await.is_err());assert!(repo.recover("default-user","arbitrary").await.is_err());
}
#[tokio::test]
async fn schema2_read_migrates_fonts_in_memory_then_explicit_save_persists_schema6() {
 let db=memory().await;let mut legacy=serde_json::to_value(AppearanceValues::default()).unwrap();
 legacy["schemaVersion"]=json!(2);legacy["typography"]["readingFollowUi"]=json!(null);
 for key in ["uiFont", "readingFont", "codeFont"] { legacy["typography"].as_object_mut().unwrap().remove(key); }
 legacy["typography"]["uiFontId"]=json!("humanist");legacy["typography"]["readingFontId"]=json!("serif");legacy["typography"]["codeFontId"]=json!("mono");
 legacy["typography"]["uiSize"]=json!(16);legacy["typography"]["readingSize"]=json!(20);legacy["typography"]["codeSize"]=json!(18);legacy["revision"]=json!(12);
 let raw=legacy.to_string();seed(&db,"appearance.snapshot.v2",&raw).await;let repo=AppearanceRepository::new(db.clone());
 let migrated=snapshot(repo.get("default-user").await.unwrap());assert_eq!(migrated.revision,12);assert_eq!(migrated.values.schema_version,6);
 assert_eq!(migrated.values.typography.ui_font, dgos_types::appearance::FontSelection::Local{family:"Segoe UI".into()});assert_eq!(migrated.values.typography.reading_font,dgos_types::appearance::FontSelection::Local{family:"Georgia".into()});assert_eq!(migrated.values.typography.code_font,dgos_types::appearance::FontSelection::SystemMono);
 assert_eq!(migrated.values.typography.ui_size,16);assert_eq!(migrated.values.typography.reading_size,20);assert_eq!(migrated.values.typography.code_size,18);assert!(!migrated.values.typography.reading_follow_ui);
 let stored=db.query_one(Statement::from_string(db.get_database_backend(),"SELECT setting_value FROM system_settings WHERE user_id='default-user' AND setting_key='appearance.snapshot.v2'")).await.unwrap().unwrap().try_get::<String>("","setting_value").unwrap();assert_eq!(stored,raw);
 let mut edited=migrated.values.clone();edited.mode="dark".into();assert!(matches!(repo.apply("default-user",&edited,12).await.unwrap(),AppearanceSave::Saved{snapshot} if snapshot.revision==13));
 let stored=db.query_one(Statement::from_string(db.get_database_backend(),"SELECT setting_value FROM system_settings WHERE user_id='default-user' AND setting_key='appearance.snapshot.v2'")).await.unwrap().unwrap().try_get::<String>("","setting_value").unwrap();let persisted:serde_json::Value=serde_json::from_str(&stored).unwrap();assert_eq!(persisted["schemaVersion"],json!(6));assert_eq!(persisted["revision"],json!(13));assert_eq!(persisted["typography"]["uiFont"]["family"],json!("Segoe UI"));
}
#[tokio::test]
async fn schema3_font_migration_preserves_local_family_intent_and_follow_setting() {
 let db=memory().await;let mut legacy=serde_json::to_value(AppearanceValues::default()).unwrap();legacy["schemaVersion"]=json!(3);legacy["revision"]=json!(21);
 let typography=legacy["typography"].as_object_mut().unwrap();
 typography.remove("uiFont");typography.remove("readingFont");typography.remove("codeFont");
 typography.insert("uiFontId".into(),json!("noto-sans-cjk-sc"));
 typography.insert("readingFontId".into(),json!("inter"));
 typography.insert("codeFontId".into(),json!("noto-sans-mono-cjk-sc"));
 typography.insert("readingFollowUi".into(),json!(true));
 seed(&db,"appearance.snapshot.v2",&legacy.to_string()).await;
 let values=snapshot(AppearanceRepository::new(db).get("default-user").await.unwrap()).values;
 assert_eq!(values.schema_version,6);assert!(values.typography.reading_follow_ui);
 assert_eq!(values.typography.ui_font,dgos_types::appearance::FontSelection::Local{family:"Noto Sans CJK SC".into()});
 assert_eq!(values.typography.reading_font,dgos_types::appearance::FontSelection::Local{family:"Inter".into()});
 assert_eq!(values.typography.code_font,dgos_types::appearance::FontSelection::Local{family:"Noto Sans Mono CJK SC".into()});
}
#[tokio::test]
async fn schema3_missing_font_intent_uses_role_defaults_without_rewriting_raw() {
 let db=memory().await;let mut legacy=serde_json::to_value(AppearanceValues::default()).unwrap();legacy["schemaVersion"]=json!(3);legacy["revision"]=json!(8);
 let typography=legacy["typography"].as_object_mut().unwrap();
 typography.remove("uiFont");typography.remove("readingFont");typography.remove("codeFont");
 typography.remove("uiFontId");typography.remove("readingFontId");typography.remove("codeFontId");
 let raw=legacy.to_string();seed(&db,"appearance.snapshot.v2",&raw).await;
 let values=snapshot(AppearanceRepository::new(db.clone()).get("default-user").await.unwrap()).values;
 assert_eq!(values.typography.ui_font,dgos_types::appearance::FontSelection::SystemUi);
 assert_eq!(values.typography.reading_font,dgos_types::appearance::FontSelection::SystemUi);
 assert_eq!(values.typography.code_font,dgos_types::appearance::FontSelection::SystemMono);
 let stored=db.query_one(Statement::from_string(db.get_database_backend(),"SELECT setting_value FROM system_settings WHERE user_id='default-user' AND setting_key='appearance.snapshot.v2'")).await.unwrap().unwrap().try_get::<String>("","setting_value").unwrap();assert_eq!(stored,raw);
}
#[tokio::test]
async fn invalid_schema4_revision_is_corrupt_and_raw_row_is_preserved() {
 let db=memory().await;let mut value=serde_json::to_value(AppearanceValues::default()).unwrap();value["schemaVersion"]=json!(4);value["revision"]=json!(-1);let raw=value.to_string();seed(&db,"appearance.snapshot.v2",&raw).await;
 let repo=AppearanceRepository::new(db.clone());assert!(matches!(repo.get("default-user").await.unwrap(),AppearanceRead::Corrupt{..}));
 let stored=db.query_one(Statement::from_string(db.get_database_backend(),"SELECT setting_value FROM system_settings WHERE user_id='default-user' AND setting_key='appearance.snapshot.v2'")).await.unwrap().unwrap().try_get::<String>("","setting_value").unwrap();assert_eq!(stored,raw);
}
#[tokio::test]
async fn invalid_schema4_leaf_is_repaired_without_rewriting_raw_row() {
 let db=memory().await;let mut value=serde_json::to_value(AppearanceValues::default()).unwrap();value["schemaVersion"]=json!(4);value["revision"]=json!(4);value["typography"]["readingFollowUi"]=json!(null);let raw=value.to_string();seed(&db,"appearance.snapshot.v2",&raw).await;
 let repo=AppearanceRepository::new(db.clone());match repo.get("default-user").await.unwrap(){AppearanceRead::Ready{snapshot,repair_warnings}=>{assert_eq!(snapshot.revision,4);assert!(snapshot.values.typography.reading_follow_ui);assert!(!repair_warnings.is_empty());},_=>panic!("expected repaired schema4")}
 let stored=db.query_one(Statement::from_string(db.get_database_backend(),"SELECT setting_value FROM system_settings WHERE user_id='default-user' AND setting_key='appearance.snapshot.v2'")).await.unwrap().unwrap().try_get::<String>("","setting_value").unwrap();assert_eq!(stored,raw);
}
#[tokio::test]
async fn schema2_typography_null_is_repaired_without_panic() {
 let db=memory().await;let mut legacy=serde_json::to_value(AppearanceValues::default()).unwrap();legacy["schemaVersion"]=json!(2);legacy["revision"]=json!(5);legacy["typography"]=json!(null);seed(&db,"appearance.snapshot.v2",&legacy.to_string()).await;
 let snapshot=snapshot(AppearanceRepository::new(db).get("default-user").await.unwrap());assert_eq!(snapshot.revision,5);assert_eq!(snapshot.values.schema_version,6);assert!(!snapshot.values.typography.reading_follow_ui);
}
#[tokio::test]
async fn appearance_valid_fields_survive_corrupt_leaves_without_rewriting_row(){
 let db=memory().await;let mut value=serde_json::to_value(AppearanceValues::default()).unwrap();value["revision"]=json!(7);value["mode"]=json!("dark");value["typography"]["codeSize"]=json!(999);value["density"]=json!(120);seed(&db,"appearance.snapshot.v2",&value.to_string()).await;
 let repo=AppearanceRepository::new(db.clone());match repo.get("default-user").await.unwrap(){AppearanceRead::Ready{snapshot,repair_warnings}=>{assert_eq!(snapshot.revision,7);assert_eq!(snapshot.values.mode,"dark");assert_eq!(snapshot.values.density,120);assert_eq!(snapshot.values.typography.code_size,12);assert!(!repair_warnings.is_empty());},_=>panic!()}
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
 let values=AppearanceValues{mode:"dark".into(),density:120,..Default::default()};
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
 let future=json!({"schemaVersion":7,"revision":9}).to_string();
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

#[tokio::test]
async fn legacy_density_migration_preserves_raw_revision_colors_and_typography_until_explicit_save() {
 for schema in [2,3,4,5] {
  for (density,expected) in [(-1000.0,60),(0.0,60),(60.0,60),(70.0,70),(74.9,70),(75.0,80),(80.0,80),(90.0,90),(100.0,100),(110.0,110),(114.9,110),(115.0,120),(120.0,120),(125.0,130),(130.0,130),(140.0,140),(150.0,150),(175.0,150),(1e100,150)] {
   let db=memory().await;let mut legacy=serde_json::to_value(AppearanceValues::default()).unwrap();
   legacy["schemaVersion"]=json!(schema);legacy["density"]=json!(density);legacy["revision"]=json!(19);
   legacy["variants"]["light"]["overrides"]["reading"]=json!({"background":"#111111","foreground":"#eeeeee"});
   legacy["typography"]["uiSize"]=json!(17);legacy["typography"]["readingSize"]=json!(21);legacy["typography"]["codeSize"]=json!(19);legacy["typography"]["readingFollowUi"]=json!(false);
   legacy["mode"]=json!("auto");legacy["effects"]=json!({"sidebarTreatment":"soft","reducedMotion":"reduce"});legacy["typography"]["lineHeight"]=json!(2.0);
   if schema < 4 {
    for field in ["uiFont","readingFont","codeFont"] {legacy["typography"].as_object_mut().unwrap().remove(field);}
    legacy["typography"]["uiFontId"]=json!("humanist");legacy["typography"]["readingFontId"]=json!("serif");legacy["typography"]["codeFontId"]=json!("mono");
   } else {
    for field in ["uiFont","readingFont","codeFont"] {legacy["typography"][field]=json!({"kind":"local","family":format!("Missing Saved {field} Family")});}
   }
   let raw=legacy.to_string();seed(&db,"appearance.snapshot.v2",&raw).await;let repo=AppearanceRepository::new(db.clone());
   let (migrated,warnings)=match repo.get("default-user").await.unwrap(){AppearanceRead::Ready{snapshot,repair_warnings}=>(snapshot,repair_warnings),_=>panic!("expected migrated row")};
   assert_eq!(migrated.revision,19);assert_eq!(migrated.values.schema_version,6);assert_eq!(migrated.values.density,expected);
   assert_eq!(warnings,if density==f64::from(expected) {vec![]} else {vec!["density_migrated".to_owned()]});
   assert_eq!(serde_json::to_value(&migrated.values.variants).unwrap(),legacy["variants"]);
   assert_eq!(migrated.values.typography.ui_size,17);assert_eq!(migrated.values.typography.reading_size,21);assert_eq!(migrated.values.typography.code_size,19);assert!(!migrated.values.typography.reading_follow_ui);
   assert_eq!(migrated.values.mode,"auto");assert_eq!(serde_json::to_value(&migrated.values.effects).unwrap(),legacy["effects"]);assert_eq!(migrated.values.typography.line_height,2.0);
   if schema>=4 {assert_eq!(serde_json::to_value(&migrated.values.typography).unwrap(),legacy["typography"]);}
   let stored=db.query_one(Statement::from_string(db.get_database_backend(),"SELECT setting_value,updated_at FROM system_settings WHERE user_id='default-user' AND setting_key='appearance.snapshot.v2'")).await.unwrap().unwrap();
   assert_eq!(stored.try_get::<String>("","setting_value").unwrap(),raw);assert_eq!(stored.try_get::<i64>("","updated_at").unwrap(),0);
   let reopened=AppearanceRepository::new(db.clone());assert_eq!(snapshot(reopened.get("default-user").await.unwrap()),migrated);
   let saved=repo.apply("default-user",&migrated.values,19).await.unwrap();assert!(matches!(saved,AppearanceSave::Saved{snapshot} if snapshot.revision==20&&snapshot.values==migrated.values));
   match reopened.get("default-user").await.unwrap(){AppearanceRead::Ready{snapshot,repair_warnings}=>{assert_eq!(snapshot.revision,20);assert_eq!(snapshot.values,migrated.values);assert!(repair_warnings.is_empty());},_=>panic!("expected persisted schema6")}
   assert!(matches!(reopened.apply("default-user",&migrated.values,19).await.unwrap(),AppearanceSave::Conflict{code:1005007,snapshot} if snapshot.revision==20&&snapshot.values==migrated.values));
  }
 }
}

#[tokio::test]
async fn invalid_density_keeps_leaf_repair_semantics_in_every_supported_schema() {
 for schema in [2,3,4,5,6] {
  let mut invalid=vec![json!(null),json!("150"),json!(true)];
  if schema==6 {invalid.extend([json!(75),json!(115),json!(125),json!(175),json!(999),json!(100.5)]);}
  for density in invalid {
   let db=memory().await;let mut raw=serde_json::to_value(AppearanceValues::default()).unwrap();raw["schemaVersion"]=json!(schema);raw["density"]=density;raw["revision"]=json!(7);
   let bytes=raw.to_string();seed(&db,"appearance.snapshot.v2",&bytes).await;
   match AppearanceRepository::new(db.clone()).get("default-user").await.unwrap(){AppearanceRead::Ready{snapshot,repair_warnings}=>{assert_eq!(snapshot.values.density,100);assert_eq!(snapshot.revision,7);assert!(repair_warnings.contains(&"/density".to_owned()));assert!(!repair_warnings.contains(&"density_migrated".to_owned()));},_=>panic!("expected repaired density")}
   let stored=db.query_one(Statement::from_string(db.get_database_backend(),"SELECT setting_value FROM system_settings WHERE user_id='default-user' AND setting_key='appearance.snapshot.v2'")).await.unwrap().unwrap().try_get::<String>("","setting_value").unwrap();assert_eq!(stored,bytes);
  }
 }
}

#[tokio::test]
async fn schema6_round_trips_every_scale_and_rejects_non_steps_without_writing() {
 let db=memory().await;let repo=AppearanceRepository::new(db.clone());
 let mut current=snapshot(repo.get("default-user").await.unwrap());
 for density in [60,70,80,90,100,110,120,130,140,150] {
  let mut values=current.values.clone();values.density=density;
  current=match repo.apply("default-user",&values,current.revision).await.unwrap(){AppearanceSave::Saved{snapshot}=>snapshot,_=>panic!("expected saved scale")};
  assert_eq!(current.values.schema_version,6);assert_eq!(current.values,values);
  assert_eq!(snapshot(repo.get("default-user").await.unwrap()),current);
 }
 let before=db.query_one(Statement::from_string(db.get_database_backend(),"SELECT setting_value,updated_at FROM system_settings WHERE user_id='default-user' AND setting_key='appearance.snapshot.v2'")).await.unwrap().unwrap();
 for density in [0,59,61,75,95,115,125,151,175] {
  let mut invalid=current.values.clone();invalid.density=density;
  assert!(repo.apply("default-user",&invalid,current.revision).await.is_err());
 }
 for schema in [2,3,4,5] {
  let mut invalid=current.values.clone();invalid.schema_version=schema;
  assert!(repo.apply("default-user",&invalid,current.revision).await.is_err());
 }
 let after=db.query_one(Statement::from_string(db.get_database_backend(),"SELECT setting_value,updated_at FROM system_settings WHERE user_id='default-user' AND setting_key='appearance.snapshot.v2'")).await.unwrap().unwrap();
 assert_eq!(after.try_get::<String>("","setting_value").unwrap(),before.try_get::<String>("","setting_value").unwrap());
 assert_eq!(after.try_get::<i64>("","updated_at").unwrap(),before.try_get::<i64>("","updated_at").unwrap());
 assert_eq!(snapshot(repo.get("default-user").await.unwrap()),current);
}

#[tokio::test]
async fn migrated_schema5_concurrent_saves_use_the_original_row_for_cas() {
 let db=memory().await;let mut legacy=serde_json::to_value(AppearanceValues::default()).unwrap();
 legacy["schemaVersion"]=json!(5);legacy["density"]=json!(115);legacy["revision"]=json!(23);
 legacy["typography"]["readingFont"]=json!({"kind":"local","family":"Missing Reading Family"});
 legacy["typography"]["readingFollowUi"]=json!(false);
 legacy["variants"]["dark"]["overrides"]["code"]=json!({"background":"#111111","foreground":"#eeeeee"});
 seed(&db,"appearance.snapshot.v2",&legacy.to_string()).await;
 let first=AppearanceRepository::new(db.clone());let second=AppearanceRepository::new(db);
 let confirmed=snapshot(first.get("default-user").await.unwrap());assert_eq!(confirmed.revision,23);assert_eq!(confirmed.values.density,120);
 let mut a=confirmed.values.clone();a.density=130;let mut b=confirmed.values.clone();b.density=110;
 let (a,b)=tokio::join!(first.apply("default-user",&a,23),second.apply("default-user",&b,23));
 let results=[a.unwrap(),b.unwrap()];assert_eq!(results.iter().filter(|result|matches!(result,AppearanceSave::Saved{..})).count(),1);
 let winner=snapshot(first.get("default-user").await.unwrap());assert_eq!(winner.revision,24);assert_eq!(winner.values.schema_version,6);
 assert_eq!(winner.values.typography,confirmed.values.typography);assert_eq!(winner.values.variants,confirmed.values.variants);
 for result in results {match result {AppearanceSave::Saved{snapshot}|AppearanceSave::Conflict{snapshot,..}=>assert_eq!(snapshot,winner)}}
}
