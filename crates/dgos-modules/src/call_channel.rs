//! Host-side binding for the module-to-host capability channel (D050, ADR-0001).
//!
//! Skeleton only: nothing outside this crate reaches the router, no Tauri
//! command carries a route, and the `host` object handed to module code
//! (D042) has no call function. It fixes what a future transport must hand
//! the dispatch layer: a request bound to a supervisor-issued runtime, its
//! verified `(module_id, version, integrity)`, the page attachment and
//! generation it was loaded into, and a host-generated `request_id`.
//!
//! A route is a routing key, not authority. It names no module, user,
//! permission or capability, and presenting it grants nothing: admission here
//! only says "this call comes from the live page instance of a ready
//! runtime". Authorization (publisher identity, manifest declaration,
//! persisted grant, enablement) is checked per call by the dispatch service
//! afterwards. Module code shares the page realm with the host (D042), so it
//! can read or forge anything the page holds; the only execution boundary is
//! admission (D041).

use crate::{
    page_loader::PageModuleLoader,
    runtime::{
        ModuleRequest, ModuleRequestOutcome, ModuleRuntimeError, ModuleRuntimeHandle,
        ModuleRuntimeState,
    },
};
use dgos_types::HostExecutionContext;
use std::{
    collections::HashMap,
    fmt,
    sync::{Mutex, MutexGuard},
};
use uuid::Uuid;

/// Maximum module calls in flight per runtime (D051). There is one capability
/// today, so this is also its per-capability limit; a second capability must
/// register its own limit before it is routed.
pub const MAX_IN_FLIGHT_CALLS: usize = 8;

/// `call_id`s remembered per route for duplicate suppression (D054). A bound,
/// not a guarantee: a resend arrives within milliseconds.
const MAX_REMEMBERED_CALL_IDS: usize = 1024;

fn valid_call_id(value: &str) -> bool {
    (1..=64).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

/// Opaque per-runtime routing key issued by the host. Not persisted, not
/// valid across attachments, runtimes or page generations.
#[derive(Clone, Eq, Hash, PartialEq)]
pub struct ModuleCallRoute(String);

impl ModuleCallRoute {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ModuleCallRoute {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ModuleCallRoute(..)")
    }
}

/// Why a call was not admitted. The channel maps all of them to the existing
/// permission error (`1004004`, D048); the reason stays host-side.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CallRejection {
    /// No such route for this attachment. Also returned when the route exists
    /// but the presented attachment differs, so the answer reveals nothing
    /// about other attachments' routes.
    UnknownRoute,
    /// The page instance is gone: the page reloaded, the attachment was
    /// superseded, the window closed, or the runtime was lost.
    Stale,
    /// The runtime is not `ready` (starting, draining, stopped, failed).
    NotReady,
    /// The same `call_id` was already delivered on this route, or the id is
    /// malformed (D054). Mapped to `1004004` like every admission failure.
    Duplicate,
    /// The runtime already has `MAX_IN_FLIGHT_CALLS` calls in flight (D051).
    /// Retryable; mapped to `1001005`, never to a permission error.
    Busy,
}

/// One admitted call. It owns the runtime request, so dropping it unsettled
/// counts as `Interrupted` (external result unknown) and never as success.
pub struct BoundModuleCall {
    runtime: ModuleRuntimeHandle,
    request: ModuleRequest,
    attachment_id: String,
    generation: u64,
}

impl BoundModuleCall {
    /// Host-generated per call; never supplied by the module.
    pub fn request_id(&self) -> &str {
        self.request.context().request_id()
    }

    /// User, `(module_id, version, integrity)` and `request_id`, all from the
    /// supervisor-issued handle.
    pub fn context(&self) -> &HostExecutionContext {
        self.request.context()
    }

    pub fn runtime(&self) -> &ModuleRuntimeHandle {
        &self.runtime
    }

    pub fn attachment_id(&self) -> &str {
        &self.attachment_id
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn finish(self, outcome: ModuleRequestOutcome) {
        self.request.finish(outcome);
    }
}

struct Route {
    runtime: ModuleRuntimeHandle,
    attachment_id: String,
    generation: u64,
    /// Recently seen `call_id`s, oldest first (D054). Bounded; discarded with
    /// the route.
    seen: std::collections::VecDeque<String>,
}

pub struct ModuleCallRouter {
    loader: PageModuleLoader,
    routes: Mutex<HashMap<String, Route>>,
}

impl ModuleCallRouter {
    pub fn new(loader: PageModuleLoader) -> Self {
        Self {
            loader,
            routes: Mutex::new(HashMap::new()),
        }
    }

    /// Issue the route for a runtime that is `ready` and loaded in the
    /// window's live attachment. A runtime has at most one route; issuing
    /// again replaces the previous one.
    pub fn issue(&self, runtime: &ModuleRuntimeHandle) -> Result<ModuleCallRoute, CallRejection> {
        self.issue_with_attachment(runtime).map(|(route, _)| route)
    }

    /// Like `issue`, also returning the attachment the route is valid for, so
    /// the transport can hand the route to exactly that page instance.
    pub fn issue_with_attachment(
        &self,
        runtime: &ModuleRuntimeHandle,
    ) -> Result<(ModuleCallRoute, String), CallRejection> {
        if runtime.state() != ModuleRuntimeState::Ready {
            return Err(CallRejection::NotReady);
        }
        let (attachment_id, generation) = self
            .loader
            .loaded_binding(runtime.runtime_id())
            .ok_or(CallRejection::Stale)?;
        let route = Uuid::new_v4().to_string();
        let mut routes = self.routes();
        // Also drop routes of runtimes that left service, so entries cannot accumulate.
        routes.retain(|_, known| {
            !known.runtime.is_same_runtime(runtime)
                && known.runtime.state() == ModuleRuntimeState::Ready
        });
        routes.insert(
            route.clone(),
            Route {
                runtime: runtime.clone(),
                attachment_id: attachment_id.clone(),
                generation,
                seen: std::collections::VecDeque::new(),
            },
        );
        Ok((ModuleCallRoute(route), attachment_id))
    }

    /// Admit one call. `attachment_id` is what the transport derived from the
    /// calling page, not a module claim; `route` is the only value the module
    /// side supplies. Re-checks the live page instance and the runtime state
    /// on every call. Does not authorize anything.
    pub fn admit(
        &self,
        route: &str,
        attachment_id: &str,
    ) -> Result<BoundModuleCall, CallRejection> {
        let (runtime, generation) = {
            let routes = self.routes();
            let known = routes
                .get(route)
                .filter(|known| known.attachment_id == attachment_id)
                .ok_or(CallRejection::UnknownRoute)?;
            (known.runtime.clone(), known.generation)
        };
        let live = self.loader.loaded_binding(runtime.runtime_id());
        if live
            .as_ref()
            .map(|(id, generation)| (id.as_str(), *generation))
            != Some((attachment_id, generation))
        {
            self.routes().remove(route);
            return Err(CallRejection::Stale);
        }
        let request = runtime
            .begin_request_limited(MAX_IN_FLIGHT_CALLS)
            .map_err(|error| match error {
                ModuleRuntimeError::AtCapacity => CallRejection::Busy,
                _ => CallRejection::NotReady,
            })?;
        Ok(BoundModuleCall {
            runtime,
            request,
            attachment_id: attachment_id.to_owned(),
            generation,
        })
    }

    /// Admit one call exactly once per `call_id` (D054). The transport can
    /// deliver one page call twice (Tauri resends over `postMessage` when the
    /// `fetch` is aborted by a navigation), and each delivery would otherwise
    /// be a separate external side effect. A repeated id on the same route is
    /// refused before a request exists; `call_id` merges nothing and is no
    /// authority, only a duplicate filter.
    pub fn admit_once(
        &self,
        route: &str,
        attachment_id: &str,
        call_id: &str,
    ) -> Result<BoundModuleCall, CallRejection> {
        self.remember_call_id(route, attachment_id, call_id)?;
        self.admit(route, attachment_id)
    }

    /// Resolve a route to its runtime for a host-side record (a crash report,
    /// D055) without opening a request. Same route, attachment, page-instance
    /// and duplicate checks as `admit_once`; the runtime's state is not
    /// required to be `ready`, since a crashed module is reported as it dies.
    pub fn resolve_once(
        &self,
        route: &str,
        attachment_id: &str,
        call_id: &str,
    ) -> Result<ModuleRuntimeHandle, CallRejection> {
        self.remember_call_id(route, attachment_id, call_id)?;
        let (runtime, generation) = {
            let routes = self.routes();
            let known = routes
                .get(route)
                .filter(|known| known.attachment_id == attachment_id)
                .ok_or(CallRejection::UnknownRoute)?;
            (known.runtime.clone(), known.generation)
        };
        let live = self.loader.loaded_binding(runtime.runtime_id());
        if live
            .as_ref()
            .map(|(id, generation)| (id.as_str(), *generation))
            != Some((attachment_id, generation))
        {
            self.routes().remove(route);
            return Err(CallRejection::Stale);
        }
        Ok(runtime)
    }

    fn remember_call_id(
        &self,
        route: &str,
        attachment_id: &str,
        call_id: &str,
    ) -> Result<(), CallRejection> {
        if !valid_call_id(call_id) {
            return Err(CallRejection::Duplicate);
        }
        let mut routes = self.routes();
        let known = routes
            .get_mut(route)
            .filter(|known| known.attachment_id == attachment_id)
            .ok_or(CallRejection::UnknownRoute)?;
        if known.seen.iter().any(|seen| seen == call_id) {
            return Err(CallRejection::Duplicate);
        }
        if known.seen.len() >= MAX_REMEMBERED_CALL_IDS {
            known.seen.pop_front();
        }
        known.seen.push_back(call_id.to_owned());
        Ok(())
    }

    /// Drop the routes of a runtime that left service. Routes of stopped or
    /// lost runtimes are already unusable; this only frees the entries.
    pub fn forget(&self, runtime: &ModuleRuntimeHandle) {
        self.routes()
            .retain(|_, known| !known.runtime.is_same_runtime(runtime));
    }

    pub fn len(&self) -> usize {
        self.routes().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn routes(&self) -> MutexGuard<'_, HashMap<String, Route>> {
        self.routes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        digest::package_integrity,
        loader::{ModuleLoader, VerifiedModuleArtifact},
        page_loader::{
            ModulePageTransport, PageInstruction, PageLoaderConfig, PageReceipt, PageSendError,
            ReceiptDisposition, RuntimeLost,
        },
    };
    use dgos_types::HostSession;
    use std::{fs, sync::Arc, time::Duration};
    use tempfile::TempDir;
    use tokio::sync::mpsc;

    #[derive(Default)]
    struct Capture {
        sent: Mutex<Vec<(String, PageInstruction)>>,
    }

    impl ModulePageTransport for Capture {
        fn send(
            &self,
            attachment: &str,
            instruction: PageInstruction,
        ) -> Result<(), PageSendError> {
            self.sent
                .lock()
                .unwrap()
                .push((attachment.to_owned(), instruction));
            Ok(())
        }
    }

    struct Fixture {
        _dir: TempDir,
        integrity: String,
        transport: Arc<Capture>,
        loader: PageModuleLoader,
        router: ModuleCallRouter,
        _lost: mpsc::UnboundedReceiver<RuntimeLost>,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fs::write(root.join("entry.js"), "export default () => ({});").unwrap();
        fs::write(
            root.join("manifest.json"),
            serde_json::json!({
                "id": "mod-a", "name": "A", "version": "1.0.0", "entry": "entry.js",
                "artifact": {"version": "1.0.0", "sha256": "0".repeat(64)},
                "permissions": [],
            })
            .to_string(),
        )
        .unwrap();
        let integrity = package_integrity(&root).unwrap();
        let transport = Arc::new(Capture::default());
        let (loader, lost) = PageModuleLoader::new(transport.clone(), PageLoaderConfig::default());
        Fixture {
            _dir: dir,
            integrity,
            router: ModuleCallRouter::new(loader.clone()),
            transport,
            loader,
            _lost: lost,
        }
    }

    impl Fixture {
        fn runtime(&self, module_id: &str) -> ModuleRuntimeHandle {
            let context = HostExecutionContext::for_module_session(
                &HostSession::local_default(),
                module_id,
                "1.0.0",
                &self.integrity,
            );
            ModuleRuntimeHandle::issue(
                context,
                VerifiedModuleArtifact::new(
                    module_id.into(),
                    "1.0.0".into(),
                    self.integrity.clone(),
                    std::sync::Arc::from(&b"export default () => ({});"[..]),
                ),
            )
        }

        /// Load `runtime` into the live attachment, answering the page's
        /// instruction with `loaded`, then mark it ready as the supervisor
        /// does.
        async fn load(&self, runtime: &ModuleRuntimeHandle) {
            let seen = self.transport.sent.lock().unwrap().len();
            let loader = self.loader.clone();
            let handle = runtime.clone();
            let task =
                tokio::spawn(
                    async move { loader.start(handle.runtime_id(), handle.artifact()).await },
                );
            let (attachment, ticket) = loop {
                if let Some((attachment, PageInstruction::Load { ticket, .. })) =
                    self.transport.sent.lock().unwrap().get(seen).cloned()
                {
                    break (attachment, ticket);
                }
                tokio::time::sleep(Duration::from_millis(2)).await;
            };
            assert_eq!(
                self.loader
                    .receive(&attachment, &ticket, PageReceipt::Loaded),
                ReceiptDisposition::Applied
            );
            task.await.unwrap().unwrap();
            runtime.mark_ready().unwrap();
        }
    }

    #[tokio::test]
    async fn an_admitted_call_is_bound_to_the_runtime_and_page_and_gets_its_own_request_id() {
        let f = fixture();
        let attachment = f.loader.attach("main").attachment_id;
        let runtime = f.runtime("mod-a");
        f.load(&runtime).await;
        let route = f.router.issue(&runtime).unwrap();

        let first = f.router.admit(route.as_str(), &attachment).unwrap();
        let second = f.router.admit(route.as_str(), &attachment).unwrap();
        assert_ne!(first.request_id(), second.request_id());
        for call in [&first, &second] {
            assert_eq!(
                call.context().module_identity(),
                Some(("mod-a", "1.0.0", f.integrity.as_str()))
            );
            assert_eq!(call.attachment_id(), attachment);
            assert_eq!(call.generation(), f.loader.generation("main"));
            assert!(call.runtime().is_same_runtime(&runtime));
        }
        assert_eq!(runtime.request_stats().in_flight, 2);
        first.finish(ModuleRequestOutcome::Succeeded);
        drop(second);
        let stats = runtime.request_stats();
        assert_eq!(
            (stats.in_flight, stats.succeeded, stats.interrupted),
            (0, 1, 1)
        );
    }

    #[tokio::test]
    async fn only_a_ready_and_loaded_runtime_gets_a_route() {
        let f = fixture();
        f.loader.attach("main");
        // Starting, never loaded.
        let starting = f.runtime("mod-a");
        assert_eq!(
            f.router.issue(&starting).err(),
            Some(CallRejection::NotReady)
        );
        // Ready by state but unknown to the page loader.
        let unloaded = f.runtime("mod-b");
        unloaded.mark_ready().unwrap();
        assert_eq!(f.router.issue(&unloaded).err(), Some(CallRejection::Stale));
        assert!(f.router.is_empty());
    }

    #[tokio::test]
    async fn an_unknown_route_or_another_attachment_is_refused_without_admitting_a_request() {
        let f = fixture();
        let attachment = f.loader.attach("main").attachment_id;
        let runtime = f.runtime("mod-a");
        f.load(&runtime).await;
        let route = f.router.issue(&runtime).unwrap();

        assert_eq!(
            f.router.admit("not-a-route", &attachment).err(),
            Some(CallRejection::UnknownRoute)
        );
        assert_eq!(
            f.router.admit(route.as_str(), "another-attachment").err(),
            Some(CallRejection::UnknownRoute)
        );
        assert_eq!(runtime.request_stats().admitted, 0);
    }

    #[tokio::test]
    async fn two_runtimes_never_resolve_to_each_others_identity() {
        let f = fixture();
        let attachment = f.loader.attach("main").attachment_id;
        let (a, b) = (f.runtime("mod-a"), f.runtime("mod-b"));
        f.load(&a).await;
        f.load(&b).await;
        let (route_a, route_b) = (f.router.issue(&a).unwrap(), f.router.issue(&b).unwrap());
        assert_ne!(route_a, route_b);
        let call_a = f.router.admit(route_a.as_str(), &attachment).unwrap();
        let call_b = f.router.admit(route_b.as_str(), &attachment).unwrap();
        assert_eq!(call_a.context().module_identity().unwrap().0, "mod-a");
        assert_eq!(call_b.context().module_identity().unwrap().0, "mod-b");
    }

    #[tokio::test]
    async fn a_page_reload_makes_the_route_stale_and_a_new_attachment_does_not_revive_it() {
        let f = fixture();
        let attachment = f.loader.attach("main").attachment_id;
        let runtime = f.runtime("mod-a");
        f.load(&runtime).await;
        let route = f.router.issue(&runtime).unwrap();
        let in_flight = f.router.admit(route.as_str(), &attachment).unwrap();

        f.loader.page_started("main");
        assert_eq!(
            f.router.admit(route.as_str(), &attachment).err(),
            Some(CallRejection::Stale)
        );
        let reattached = f.loader.attach("main").attachment_id;
        assert_eq!(
            f.router.admit(route.as_str(), &reattached).err(),
            Some(CallRejection::UnknownRoute)
        );
        assert!(f.router.is_empty(), "the stale route is dropped");
        // The call admitted before the reload keeps its own terminal outcome.
        assert_eq!(runtime.request_stats().in_flight, 1);
        in_flight.finish(ModuleRequestOutcome::Failed);
        assert_eq!(runtime.request_stats().in_flight, 0);
    }

    #[tokio::test]
    async fn superseding_the_attachment_without_a_reload_also_stales_the_route() {
        let f = fixture();
        let attachment = f.loader.attach("main").attachment_id;
        let runtime = f.runtime("mod-a");
        f.load(&runtime).await;
        let route = f.router.issue(&runtime).unwrap();
        f.loader.attach("main");
        assert_eq!(
            f.router.admit(route.as_str(), &attachment).err(),
            Some(CallRejection::Stale)
        );
    }

    #[tokio::test]
    async fn a_runtime_that_is_not_ready_admits_nothing_and_a_restart_needs_a_new_route() {
        let f = fixture();
        let attachment = f.loader.attach("main").attachment_id;
        let runtime = f.runtime("mod-a");
        f.load(&runtime).await;
        let route = f.router.issue(&runtime).unwrap();

        runtime.begin_draining().unwrap();
        assert_eq!(
            f.router.admit(route.as_str(), &attachment).err(),
            Some(CallRejection::NotReady)
        );
        assert_eq!(runtime.request_stats().rejected, 1);
        assert_eq!(runtime.request_stats().admitted, 0);

        // A restarted runtime is a different instance: the old route never
        // reaches it, and the new one needs its own route.
        let restarted = f.runtime("mod-a");
        f.load(&restarted).await;
        let new_route = f.router.issue(&restarted).unwrap();
        assert_ne!(route, new_route);
        let call = f.router.admit(new_route.as_str(), &attachment).unwrap();
        assert!(call.runtime().is_same_runtime(&restarted));
    }

    #[tokio::test]
    async fn issuing_again_replaces_the_runtimes_previous_route_and_forget_drops_it() {
        let f = fixture();
        let attachment = f.loader.attach("main").attachment_id;
        let runtime = f.runtime("mod-a");
        f.load(&runtime).await;
        let first = f.router.issue(&runtime).unwrap();
        let second = f.router.issue(&runtime).unwrap();
        assert_eq!(f.router.len(), 1);
        assert_eq!(
            f.router.admit(first.as_str(), &attachment).err(),
            Some(CallRejection::UnknownRoute)
        );
        f.router.forget(&runtime);
        assert!(f.router.is_empty());
        assert_eq!(
            f.router.admit(second.as_str(), &attachment).err(),
            Some(CallRejection::UnknownRoute)
        );
    }

    #[tokio::test]
    async fn in_flight_calls_are_capped_per_runtime_and_a_refusal_is_not_admission() {
        let f = fixture();
        let attachment = f.loader.attach("main").attachment_id;
        let runtime = f.runtime("mod-a");
        let other = f.runtime("mod-b");
        f.load(&runtime).await;
        f.load(&other).await;
        let route = f.router.issue(&runtime).unwrap();
        let other_route = f.router.issue(&other).unwrap();

        let held: Vec<_> = (0..MAX_IN_FLIGHT_CALLS)
            .map(|_| f.router.admit(route.as_str(), &attachment).unwrap())
            .collect();
        assert_eq!(
            f.router.admit(route.as_str(), &attachment).err(),
            Some(CallRejection::Busy)
        );
        let stats = runtime.request_stats();
        assert_eq!(stats.admitted as usize, MAX_IN_FLIGHT_CALLS);
        assert_eq!(stats.in_flight, MAX_IN_FLIGHT_CALLS);
        assert_eq!(stats.rejected, 1);
        // The cap belongs to the runtime: another module is unaffected.
        assert!(f.router.admit(other_route.as_str(), &attachment).is_ok());

        // Settling one frees exactly one slot, and a refused call is retryable.
        let mut held = held;
        held.pop().unwrap().finish(ModuleRequestOutcome::Succeeded);
        let refilled = f.router.admit(route.as_str(), &attachment);
        assert!(refilled.is_ok());
        assert_eq!(
            f.router.admit(route.as_str(), &attachment).err(),
            Some(CallRejection::Busy)
        );
    }

    #[tokio::test]
    async fn concurrent_admissions_never_exceed_the_cap() {
        let f = Arc::new(fixture());
        let attachment = f.loader.attach("main").attachment_id;
        let runtime = f.runtime("mod-a");
        f.load(&runtime).await;
        let route = f.router.issue(&runtime).unwrap();
        let tasks: Vec<_> = (0..MAX_IN_FLIGHT_CALLS * 4)
            .map(|_| {
                let (f, route, attachment) = (f.clone(), route.clone(), attachment.clone());
                tokio::spawn(async move { f.router.admit(route.as_str(), &attachment) })
            })
            .collect();
        let mut admitted = Vec::new();
        let mut busy = 0;
        for task in tasks {
            match task.await.unwrap() {
                Ok(call) => admitted.push(call),
                Err(CallRejection::Busy) => busy += 1,
                Err(other) => panic!("unexpected rejection {other:?}"),
            }
        }
        assert_eq!(admitted.len(), MAX_IN_FLIGHT_CALLS);
        assert_eq!(busy, MAX_IN_FLIGHT_CALLS * 3);
        assert_eq!(runtime.request_stats().in_flight, MAX_IN_FLIGHT_CALLS);
    }

    #[tokio::test]
    async fn a_repeated_call_id_on_a_route_is_dropped_before_a_request_exists() {
        let f = fixture();
        let attachment = f.loader.attach("main").attachment_id;
        let runtime = f.runtime("mod-a");
        f.load(&runtime).await;
        let route = f.router.issue(&runtime).unwrap();

        let first = f
            .router
            .admit_once(route.as_str(), &attachment, "call-1")
            .unwrap();
        // The transport's resend of the same page call.
        assert_eq!(
            f.router
                .admit_once(route.as_str(), &attachment, "call-1")
                .err(),
            Some(CallRejection::Duplicate)
        );
        let stats = runtime.request_stats();
        assert_eq!((stats.admitted, stats.rejected, stats.in_flight), (1, 0, 1));

        // A different id is a different call; a settled call's id stays used.
        first.finish(ModuleRequestOutcome::Succeeded);
        assert!(f
            .router
            .admit_once(route.as_str(), &attachment, "call-2")
            .is_ok());
        assert_eq!(
            f.router
                .admit_once(route.as_str(), &attachment, "call-1")
                .err(),
            Some(CallRejection::Duplicate)
        );
    }

    #[tokio::test]
    async fn call_ids_are_scoped_to_their_route_and_malformed_ids_never_admit() {
        let f = fixture();
        let attachment = f.loader.attach("main").attachment_id;
        let (a, b) = (f.runtime("mod-a"), f.runtime("mod-b"));
        f.load(&a).await;
        f.load(&b).await;
        let (route_a, route_b) = (f.router.issue(&a).unwrap(), f.router.issue(&b).unwrap());

        assert!(f
            .router
            .admit_once(route_a.as_str(), &attachment, "same")
            .is_ok());
        assert!(f
            .router
            .admit_once(route_b.as_str(), &attachment, "same")
            .is_ok());
        for bad in ["", "has space", "semi;colon", &"x".repeat(65), "naïve"] {
            assert_eq!(
                f.router
                    .admit_once(route_a.as_str(), &attachment, bad)
                    .err(),
                Some(CallRejection::Duplicate),
                "{bad:?}"
            );
        }
        // An unknown route is answered before the id is looked at, so a probe
        // cannot learn which ids a route has seen.
        assert_eq!(
            f.router.admit_once("nope", &attachment, "fresh").err(),
            Some(CallRejection::UnknownRoute)
        );
        assert_eq!(a.request_stats().admitted, 1);
    }

    #[tokio::test]
    async fn concurrent_deliveries_of_one_call_id_admit_exactly_one() {
        let f = Arc::new(fixture());
        let attachment = f.loader.attach("main").attachment_id;
        let runtime = f.runtime("mod-a");
        f.load(&runtime).await;
        let route = f.router.issue(&runtime).unwrap();
        let tasks: Vec<_> = (0..16)
            .map(|_| {
                let (f, route, attachment) = (f.clone(), route.clone(), attachment.clone());
                tokio::spawn(async move { f.router.admit_once(route.as_str(), &attachment, "one") })
            })
            .collect();
        let mut admitted = Vec::new();
        let mut duplicates = 0;
        for task in tasks {
            match task.await.unwrap() {
                Ok(call) => admitted.push(call),
                Err(CallRejection::Duplicate) => duplicates += 1,
                Err(other) => panic!("unexpected rejection {other:?}"),
            }
        }
        assert_eq!((admitted.len(), duplicates), (1, 15));
        assert_eq!(runtime.request_stats().admitted, 1);
    }

    #[tokio::test]
    async fn only_the_most_recent_call_ids_are_remembered() {
        let f = fixture();
        let attachment = f.loader.attach("main").attachment_id;
        let runtime = f.runtime("mod-a");
        f.load(&runtime).await;
        let route = f.router.issue(&runtime).unwrap();
        for n in 0..=MAX_REMEMBERED_CALL_IDS {
            let call = f
                .router
                .admit_once(route.as_str(), &attachment, &format!("id-{n}"))
                .unwrap();
            call.finish(ModuleRequestOutcome::Succeeded);
        }
        // The oldest id fell out of the bound; the newest is still remembered.
        assert!(f
            .router
            .admit_once(route.as_str(), &attachment, "id-0")
            .is_ok());
        let newest = format!("id-{MAX_REMEMBERED_CALL_IDS}");
        assert_eq!(
            f.router
                .admit_once(route.as_str(), &attachment, &newest)
                .err(),
            Some(CallRejection::Duplicate)
        );
    }
}
