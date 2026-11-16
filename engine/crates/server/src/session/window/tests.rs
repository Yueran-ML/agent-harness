use super::*;
use crate::presentation::{LayerSnapshot, Origin};
use crate::session::GESTURE_IDLE;
use columbina_model::grade::{
    Interp, Key, Node, NodeKind, Param, Target, Window, WindowPoint, WindowShape,
};
use columbina_model::{History, Op};
use columbina_view::window_overlay::Control;

#[cfg(windows)]
#[allow(
    unsafe_code,
    reason = "hidden test-owned Win32 parent and synthetic pointer messages"
)]
mod ownership {
    use super::*;
    use std::future::Future;
    use std::pin::Pin as FuturePin;
    use std::task::Poll;
    use windows::core::w;
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, DispatchMessageW, FindWindowExW, GetMessageW, PostMessageW,
        PostThreadMessageW, MSG, WINDOW_EX_STYLE, WM_LBUTTONDOWN, WM_MOUSEMOVE, WM_QUIT, WS_POPUP,
    };

    struct Parent {
        hwnd: isize,
        thread_id: u32,
        thread: Option<std::thread::JoinHandle<()>>,
    }
    impl Parent {
        fn down(&self) {
            self.pointer(WM_LBUTTONDOWN, 64);
        }

        fn pointer(&self, message: u32, x: i32) {
            unsafe {
                let child = FindWindowExW(
                    Some(HWND(self.hwnd as *mut _)),
                    None,
                    w!("ColumbinaView"),
                    None,
                )
                .unwrap();
                PostMessageW(
                    Some(child),
                    message,
                    WPARAM(1),
                    LPARAM((x | (36 << 16)) as isize),
                )
                .unwrap();
            }
        }

        fn open() -> Self {
            let (send, recv) = std::sync::mpsc::channel();
            let thread = std::thread::spawn(move || unsafe {
                let hwnd = CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    w!("STATIC"),
                    w!("Window ownership test"),
                    WS_POPUP,
                    0,
                    0,
                    128,
                    72,
                    None,
                    None,
                    None,
                    None,
                )
                .unwrap();
                send.send((hwnd.0 as isize, GetCurrentThreadId())).unwrap();
                let mut message = MSG::default();
                while GetMessageW(&raw mut message, None, 0, 0).0 > 0 {
                    DispatchMessageW(&raw const message);
                }
                DestroyWindow(hwnd).unwrap();
            });
            let (hwnd, thread_id) = recv.recv().unwrap();
            Self {
                hwnd,
                thread_id,
                thread: Some(thread),
            }
        }
    }
    impl Drop for Parent {
        fn drop(&mut self) {
            unsafe {
                PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0)).unwrap();
            }
            self.thread.take().unwrap().join().unwrap();
        }
    }

    fn connection(id: u64) -> (Connection, crate::queue::Receiver) {
        let (sender, receiver) = crate::queue::queue(crate::queue::Limits {
            messages: 100,
            bytes: 1 << 20,
        });
        (
            Connection::new(id, sender, tokio::sync::watch::channel(false).0),
            receiver,
        )
    }

    async fn pending<T>(mut future: FuturePin<&mut impl Future<Output = T>>) {
        assert!(
            std::future::poll_fn(|cx| Poll::Ready(future.as_mut().poll(cx)))
                .await
                .is_pending()
        );
    }

    async fn attached() -> (Parent, crate::view::Viewer, Mutex<State>, Selection) {
        let parent = Parent::open();
        let viewer = crate::view::Viewer::new(false, crate::gpu::Graphics::new());
        let attach = serde_json::value::to_raw_value(&json!({"monitor":"timeline","parent":parent.hwnd.to_string(),"x":0,"y":0,"width":128,"height":72})).unwrap();
        viewer
            .call(Kind::Desktop, "view.attach", Some(&attach))
            .await
            .unwrap();
        let (mut state, _, _, _) = fixture();
        let selection = state.window.selection.as_ref().unwrap().as_ref().clone();
        state.window_unbind("fixture");
        (parent, viewer, Mutex::new(state), selection)
    }

    async fn native_engine() -> (tempfile::TempDir, Parent, Engine, Selection) {
        let dir = tempfile::tempdir().unwrap();
        let parent = Parent::open();
        let (state, _, _, _) = fixture();
        let selection = state.window.selection.as_ref().unwrap().as_ref().clone();
        let mut project = state.history.project().clone();
        project.timelines[0].tracks.truncate(1);
        project.timelines[0].tracks[0].clips[0].source = columbina_model::Source::Color {
            color: columbina_model::Rgba([80, 40, 20, 255]),
        };
        project.media.clear();
        let (store, _) = columbina_store::Store::open(dir.path()).unwrap();
        let saver = columbina_store::Saver::new(store, |_| {}).unwrap();
        let engine = Engine::new(
            History::new(project),
            saver,
            dir.path().to_string_lossy().into_owned(),
            crate::queue::Limits {
                messages: 100,
                bytes: 1 << 20,
            },
            None,
            false,
            crate::transport::Setup {
                sound: crate::audio::Kind::Null,
                fail_card_at: None,
                test_hooks: true,
            },
            &crate::proxy::Setup {
                make: false,
                ..crate::proxy::Setup::default()
            },
        );
        let attach = serde_json::value::to_raw_value(&json!({"monitor":"timeline","parent":parent.hwnd.to_string(),"x":0,"y":0,"width":128,"height":72})).unwrap();
        engine
            .viewer
            .call(Kind::Desktop, "view.attach", Some(&attach))
            .await
            .unwrap();
        let load = serde_json::value::to_raw_value(&json!({"monitor":"timeline","timeline":"t1"}))
            .unwrap();
        let loading = {
            let state = engine.state.lock().await;
            engine
                .transport
                .begin_load(state.history.project(), 0, Some(&load))
                .unwrap()
        };
        loading.done().await.unwrap();
        (dir, parent, engine, selection)
    }

    async fn note(receiver: &mut crate::queue::Receiver, method: &str) -> Value {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let line = receiver.recv().await.unwrap();
                receiver.wrote(1);
                let value: Value = serde_json::from_slice(line.as_ref()).unwrap();
                if value["method"] == method {
                    return value["params"].clone();
                }
            }
        })
        .await
        .unwrap()
    }

    async fn input(engine: &Engine, connection: &Connection, value: Value) {
        let id = RawValue::from_string("1".into()).unwrap();
        let params = serde_json::value::to_raw_value(&value).unwrap();
        assert!(call(
            engine,
            connection,
            Kind::Desktop,
            &id,
            "window.input",
            Some(&params)
        )
        .await
        .is_ok());
    }

    async fn wait_selection(engine: &Engine, binding: u64) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let current = engine
                .viewer
                .presentation_snapshot(Monitor::Timeline)
                .and_then(|(instance, shown)| {
                    engine
                        .viewer
                        .frames
                        .observe(Monitor::Timeline, instance, shown)
                });
            if current.is_some_and(|frame| {
                frame.ready
                    && frame
                        .selection
                        .as_ref()
                        .is_some_and(|s| s.binding == binding)
            }) {
                break;
            }
            assert!(Instant::now() < deadline, "selected frame not presented");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    #[tokio::test]
    async fn disconnect_retires_native_capture_before_waiting_for_view_cleanup() {
        let (_dir, parent, engine, selection) = native_engine().await;
        let id = engine.arrive(tokio::sync::watch::channel(false).0).unwrap();
        let (a, mut a_notes) = connection(id);
        let (b, _b_notes) = connection(8);
        engine.state.lock().await.peers.insert(
            id,
            crate::session::Peer {
                kind: Kind::Desktop,
                queue: a.sender().clone(),
                stop: a.stop.clone(),
            },
        );
        let bound = bind(&engine.state, &engine.viewer, &a, Kind::Desktop, selection)
            .await
            .unwrap();
        wait_selection(&engine, bound["binding"].as_u64().unwrap()).await;
        parent.down();
        let down = note(&mut a_notes, "view.pointer").await;
        input(
            &engine,
            &a,
            json!({"binding":bound["binding"],"kind":"down","event":down["event"]}),
        )
        .await;
        loop {
            if note(&mut a_notes, "window.state").await["phase"] == "armed" {
                break;
            }
        }
        let (capture, pin) = {
            let state = engine.state.lock().await;
            let capture = state.window.active.as_ref().unwrap();
            (capture.id, capture.pin.clone())
        };
        parent.pointer(WM_MOUSEMOVE, 72);
        let moved = note(&mut a_notes, "view.pointer").await;
        input(&engine, &a, json!({"binding":bound["binding"],"capture":capture,"kind":"move","event":moved["event"]})).await;
        let (revision, selection) = {
            let state = engine.state.lock().await;
            assert_eq!(state.history.revision(), 1);
            assert!(state.history.gesture().is_some());
            (
                state.history.revision(),
                state.window.selection.as_ref().unwrap().as_ref().clone(),
            )
        };
        let busy = engine.viewer.test_busy().await;
        let mut next = Box::pin(bind(
            &engine.state,
            &engine.viewer,
            &b,
            Kind::Desktop,
            selection,
        ));
        pending(next.as_mut()).await;
        let mut leaving = Box::pin(engine.leave(id));
        pending(leaving.as_mut()).await;
        {
            let state = engine.state.lock().await;
            assert!(
                state.window.active.is_none(),
                "disconnected capture must end before waiting for view busy"
            );
            assert!(!pin.live.load(Ordering::Acquire));
            assert!(state.history.gesture().is_none());
            assert!(!state.peers.contains_key(&id));
        }
        parent.pointer(WM_MOUSEMOVE, 88);
        note(&mut a_notes, "view.pointer").await;
        assert_eq!(
            engine.state.lock().await.history.revision(),
            revision,
            "native moves during delayed cleanup cannot write for the disconnected owner"
        );
        drop(busy);
        let (next, ()) = tokio::join!(next, leaving);
        next.unwrap();
        assert_eq!(engine.state.lock().await.window.owner, Some(b.id));
        assert_eq!(
            engine.viewer.test_pointer_owner(Monitor::Timeline),
            Some(b.id)
        );
        engine.stop_background();
    }

    #[tokio::test]
    async fn retired_binding_cleanup_releases_its_native_pointer_and_sink() {
        let (_parent, viewer, state, selection) = attached().await;
        let (a, _a_notes) = connection(7);
        let bound = bind(&state, &viewer, &a, Kind::Desktop, selection)
            .await
            .unwrap();
        disconnect_window_grade(&mut *state.lock().await);
        assert!(state.lock().await.window.owner.is_none());
        assert_eq!(viewer.test_pointer_owner(Monitor::Timeline), Some(a.id));
        let native_pointer = || {
            viewer
                .slot(Monitor::Timeline)
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .pointer()
        };
        assert_eq!(native_pointer(), Some(columbina_view::Cursor::Move));
        unbind(
            &state,
            &viewer,
            &a,
            Kind::Desktop,
            bound["binding"].as_u64().unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(native_pointer(), None);
        assert_eq!(viewer.test_pointer_owner(Monitor::Timeline), None);
        assert!(state.lock().await.window.pointer_binding.is_none());
    }

    fn native_pointer(viewer: &crate::view::Viewer) -> Option<columbina_view::Cursor> {
        viewer
            .slot(Monitor::Timeline)
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .pointer()
    }

    async fn ordinary_pointer(engine: &Engine, connection: &Connection, value: Value) -> bool {
        let id = RawValue::from_string("1".into()).unwrap();
        let params = serde_json::value::to_raw_value(&value).unwrap();
        crate::session::view_pointer(engine, connection, Kind::Desktop, &id, Some(&params))
            .await
            .is_ok()
    }

    #[tokio::test]
    async fn retired_cleanup_cannot_disable_a_new_window_binding_even_for_the_same_connection() {
        for successor in [7, 8] {
            let (_parent, viewer, state, mut selection) = attached().await;
            let (a, _a_notes) = connection(7);
            let (b, _b_notes) = connection(successor);
            let old = bind(&state, &viewer, &a, Kind::Desktop, selection.clone())
                .await
                .unwrap();
            {
                let mut state = state.lock().await;
                disconnect_window_grade(&mut state);
                state.apply_worked_out_locked(Kind::Agent, None, None, Instant::now(), |_| {
                    Ok(vec![json!({"op":"connect","grade":{"level":"clip","timeline":"t1","clip":"c1"},"link":{"from":"source","to":"n1","kind":"rgb","input":0}})])
                }).unwrap();
                selection.revision = state.history.revision();
            }
            let new = bind(&state, &viewer, &b, Kind::Desktop, selection)
                .await
                .unwrap();
            assert!(unbind(
                &state,
                &viewer,
                &a,
                Kind::Desktop,
                old["binding"].as_u64().unwrap()
            )
            .await
            .is_err());
            assert_eq!(native_pointer(&viewer), Some(columbina_view::Cursor::Move));
            assert_eq!(
                viewer.test_pointer_owner(Monitor::Timeline),
                Some(successor)
            );
            let state = state.lock().await;
            assert_eq!(
                state.window.selection.as_ref().unwrap().binding,
                new["binding"].as_u64().unwrap()
            );
            assert_eq!(
                state.window.pointer_binding,
                Some((successor, new["binding"].as_u64().unwrap()))
            );
        }
    }

    #[tokio::test]
    async fn ordinary_pointer_takeover_before_or_after_retirement_revokes_old_cleanup() {
        for before_retirement in [true, false] {
            for successor in [7, 8] {
                let (_dir, _parent, engine, selection) = native_engine().await;
                let (a, _a_notes) = connection(7);
                let (b, _b_notes) = connection(successor);
                let old = bind(&engine.state, &engine.viewer, &a, Kind::Desktop, selection)
                    .await
                    .unwrap();
                let pointer = json!({"monitor":"timeline","on":true,"cursor":"crosshair"});
                if before_retirement {
                    assert!(ordinary_pointer(&engine, &b, pointer.clone()).await);
                }
                disconnect_window_grade(&mut *engine.state.lock().await);
                if !before_retirement {
                    assert!(ordinary_pointer(&engine, &b, pointer).await);
                }
                assert!(engine.state.lock().await.window.pointer_binding.is_none());
                assert!(unbind(
                    &engine.state,
                    &engine.viewer,
                    &a,
                    Kind::Desktop,
                    old["binding"].as_u64().unwrap()
                )
                .await
                .is_err());
                assert_eq!(
                    native_pointer(&engine.viewer),
                    Some(columbina_view::Cursor::Crosshair)
                );
                assert_eq!(
                    engine.viewer.test_pointer_owner(Monitor::Timeline),
                    Some(successor)
                );
                engine.stop_background();
            }
        }
    }

    #[tokio::test]
    async fn failed_timeline_and_successful_source_pointer_calls_preserve_cleanup_authority() {
        let (_dir, _parent, engine, selection) = native_engine().await;
        let (a, _a_notes) = connection(7);
        let bound = bind(&engine.state, &engine.viewer, &a, Kind::Desktop, selection)
            .await
            .unwrap();
        let binding = bound["binding"].as_u64().unwrap();
        assert!(
            !ordinary_pointer(
                &engine,
                &a,
                json!({"monitor":"timeline","on":true,"cursor":"invalid"})
            )
            .await
        );
        assert!(ordinary_pointer(&engine, &a, json!({"monitor":"source","on":false})).await);
        assert_eq!(
            engine.state.lock().await.window.pointer_binding,
            Some((a.id, binding))
        );
        disconnect_window_grade(&mut *engine.state.lock().await);
        unbind(&engine.state, &engine.viewer, &a, Kind::Desktop, binding)
            .await
            .unwrap();
        assert_eq!(native_pointer(&engine.viewer), None);
        assert_eq!(engine.viewer.test_pointer_owner(Monitor::Timeline), None);
        engine.stop_background();
    }

    #[tokio::test]
    async fn binding_retiring_during_native_unbind_preparation_stays_released() {
        let (_parent, viewer, state, selection) = attached().await;
        let (a, _a_notes) = connection(7);
        let bound = bind(&state, &viewer, &a, Kind::Desktop, selection)
            .await
            .unwrap();
        let binding = bound["binding"].as_u64().unwrap();
        let params =
            serde_json::value::to_raw_value(&json!({"monitor":"timeline","on":false})).unwrap();
        let change = viewer
            .prepare_pointer(Kind::Desktop, a.id, a.sender(), Some(&params))
            .await
            .unwrap();
        state
            .lock()
            .await
            .window_check_binding(a.id, binding)
            .unwrap();
        assert!(change.owned_by(a.id));
        change.apply().await.unwrap();
        assert_eq!(native_pointer(&viewer), None);
        disconnect_window_grade(&mut *state.lock().await);
        finish_unbind(&state, &change, a.id, binding, true)
            .await
            .unwrap();
        assert_eq!(
            native_pointer(&viewer),
            None,
            "logical retirement must not roll back native release"
        );
        assert_eq!(viewer.test_pointer_owner(Monitor::Timeline), None);
        assert!(state.lock().await.window.pointer_binding.is_none());
    }

    #[tokio::test]
    async fn queued_old_unbind_cannot_disable_the_next_binding_owner() {
        let (_parent, viewer, state, selection) = attached().await;
        let (a, _a_notes) = connection(7);
        let (b, _b_notes) = connection(8);
        let first = bind(&state, &viewer, &a, Kind::Desktop, selection.clone())
            .await
            .unwrap();
        let busy = viewer.test_busy().await;
        let mut next = Box::pin(bind(&state, &viewer, &b, Kind::Desktop, selection));
        pending(next.as_mut()).await;
        let mut old_clear = Box::pin(unbind(
            &state,
            &viewer,
            &a,
            Kind::Desktop,
            first["binding"].as_u64().unwrap(),
        ));
        pending(old_clear.as_mut()).await;
        drop(busy);
        let (next, _old_clear) = tokio::join!(next, old_clear);
        next.unwrap();
        assert_eq!(state.lock().await.window.owner, Some(8));
        assert_eq!(viewer.test_pointer_owner(Monitor::Timeline), Some(8));
    }

    #[tokio::test]
    async fn queued_bind_and_clear_operations_commit_in_view_order() {
        let (_parent, viewer, state, selection) = attached().await;
        let (a, _a_notes) = connection(7);
        let (b, _b_notes) = connection(8);
        let (c, _c_notes) = connection(9);
        for visible in [true, false] {
            bind(&state, &viewer, &a, Kind::Desktop, selection.clone())
                .await
                .unwrap();
            let busy = viewer.test_busy().await;
            let mut middle = selection.clone();
            middle.visible = visible;
            let mut first = Box::pin(bind(&state, &viewer, &b, Kind::Desktop, middle));
            let mut second = Box::pin(bind(&state, &viewer, &c, Kind::Desktop, selection.clone()));
            pending(first.as_mut()).await;
            pending(second.as_mut()).await;
            let locked = state.lock().await;
            drop(busy);
            pending(first.as_mut()).await;
            pending(second.as_mut()).await;
            assert_eq!(locked.window.owner, Some(7));
            assert_eq!(
                viewer.test_pointer_owner(Monitor::Timeline),
                Some(7),
                "a later bind cannot change the pointer while the earlier turn is waiting for State"
            );
            drop(locked);
            let (first, second) = tokio::join!(first, second);
            assert!(first.unwrap()["binding"].as_u64() < second.unwrap()["binding"].as_u64());
            assert_eq!(state.lock().await.window.owner, Some(9));
            assert_eq!(viewer.test_pointer_owner(Monitor::Timeline), Some(9));
        }
        // A clear that queues before the new bind must also finish its native work first.
        let binding = state
            .lock()
            .await
            .window
            .selection
            .as_ref()
            .unwrap()
            .binding;
        let busy = viewer.test_busy().await;
        let mut clear = Box::pin(unbind(&state, &viewer, &c, Kind::Desktop, binding));
        pending(clear.as_mut()).await;
        let mut next = Box::pin(bind(&state, &viewer, &b, Kind::Desktop, selection.clone()));
        pending(next.as_mut()).await;
        drop(busy);
        let (clear, next) = tokio::join!(clear, next);
        clear.unwrap();
        next.unwrap();
        assert_eq!(state.lock().await.window.owner, Some(8));
        assert_eq!(viewer.test_pointer_owner(Monitor::Timeline), Some(8));

        let busy = viewer.test_busy().await;
        let mut forget = Box::pin(viewer.forget(8));
        pending(forget.as_mut()).await;
        let mut next = Box::pin(bind(&state, &viewer, &c, Kind::Desktop, selection));
        pending(next.as_mut()).await;
        drop(busy);
        let ((), next) = tokio::join!(forget, next);
        next.unwrap();
        assert_eq!(state.lock().await.window.owner, Some(9));
        assert_eq!(viewer.test_pointer_owner(Monitor::Timeline), Some(9));
    }

    #[tokio::test]
    async fn revision_change_while_bind_waits_keeps_the_original_owner_and_native_down() {
        let (parent, viewer, state, selection) = attached().await;
        let (a, mut a_notes) = connection(7);
        let (b, mut b_notes) = connection(8);
        let original = bind(&state, &viewer, &a, Kind::Desktop, selection.clone())
            .await
            .unwrap();
        let busy = viewer.test_busy().await;
        let mut next = Box::pin(bind(&state, &viewer, &b, Kind::Desktop, selection));
        pending(next.as_mut()).await;
        state.lock().await.apply_worked_out_locked(Kind::Desktop, None, None, Instant::now(), |_| {
            Ok(vec![json!({"op":"set_track","timeline":"t1","id":"V1","name":"changed while binding waited"})])
        }).unwrap();
        drop(busy);
        assert!(next.await.is_err());
        let locked = state.lock().await;
        assert_eq!(locked.window.owner, Some(7));
        assert_eq!(
            locked.window.selection.as_ref().unwrap().binding,
            original["binding"].as_u64().unwrap()
        );
        assert_eq!(
            locked.window.pointer_binding,
            Some((7, original["binding"].as_u64().unwrap()))
        );
        drop(locked);
        assert_eq!(viewer.test_pointer_owner(Monitor::Timeline), Some(7));
        parent.down();
        let line = tokio::time::timeout(Duration::from_secs(3), a_notes.recv())
            .await
            .unwrap()
            .unwrap();
        let note: Value = serde_json::from_slice(line.as_ref()).unwrap();
        assert_eq!(note["method"], "view.pointer");
        assert_eq!(note["params"]["kind"], "down");
        assert!(b_notes.try_recv().is_none());
    }

    #[tokio::test]
    async fn revision_change_after_native_preparation_restores_cursor_and_original_owner() {
        let (_parent, viewer, state, selection) = attached().await;
        let (a, _a_notes) = connection(7);
        let (b, _b_notes) = connection(8);
        bind(&state, &viewer, &a, Kind::Desktop, selection.clone())
            .await
            .unwrap();
        let params =
            serde_json::value::to_raw_value(&json!({"monitor":"timeline","on":false})).unwrap();
        let change = viewer
            .prepare_pointer(Kind::Desktop, b.id, b.sender(), Some(&params))
            .await
            .unwrap();
        change.apply().await.unwrap();
        let native_cursor = || {
            viewer
                .slot(Monitor::Timeline)
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .pointer()
        };
        assert_eq!(native_cursor(), None);
        assert_eq!(viewer.test_pointer_owner(Monitor::Timeline), Some(7));
        state.lock().await.apply_worked_out_locked(Kind::Desktop, None, None, Instant::now(), |_| {
            Ok(vec![json!({"op":"set_track","timeline":"t1","id":"V1","name":"changed after native preparation"})])
        }).unwrap();
        let mut hidden = selection;
        hidden.visible = false;
        assert!(finish_bind(&state, &change, b.id, hidden).await.is_err());
        assert_eq!(native_cursor(), Some(columbina_view::Cursor::Move));
        assert_eq!(state.lock().await.window.owner, Some(7));
        assert_eq!(viewer.test_pointer_owner(Monitor::Timeline), Some(7));
        assert_eq!(
            state
                .lock()
                .await
                .window
                .pointer_binding
                .map(|(owner, _)| owner),
            Some(7)
        );
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "one coherent project, native down and selected frame fixture"
)]
fn fixture() -> (State, Registry, Arc<Pin>, Arc<FrameSnapshot>) {
    let mut state = crate::session::tests::state();
    let mut project = state.history.project().clone();
    let mut node = Node::corrector("n1");
    let NodeKind::Corrector(c) = &mut node.kind else {
        unreachable!()
    };
    let mut window = Window::new("w1", WindowShape::circular());
    window.transform.pan = Param::Keys(vec![
        Key {
            frame: 0,
            value: 50.0,
            interp: Interp::Bezier {
                out: [0.2, 1.0],
                into: [0.4, -2.0],
            },
        },
        Key {
            frame: 20,
            value: 70.0,
            interp: Interp::Hold,
        },
    ]);
    c.windows = vec![window, Window::new("w2", WindowShape::curve())];
    project.timelines[0].width = 128;
    project.timelines[0].height = 72;
    project.timelines[0].tracks[0].clips[0]
        .grade
        .nodes
        .push(node);
    project.timelines[0].tracks[0].clips[0].grade.links = vec![
        columbina_model::grade::Link::rgb("source", "n1"),
        columbina_model::grade::Link::rgb("n1", "output"),
    ];
    state.history = History::new(project);
    let selection = Arc::new(Selection {
        monitor: Monitor::Timeline,
        revision: 0,
        timeline: "t1".into(),
        target: Target::Clip {
            timeline: "t1".into(),
            clip: "c1".into(),
        },
        clip: Some("c1".into()),
        node: "n1".into(),
        selected: Some("w1".into()),
        visible: true,
        outline: true,
        tool: Tool::Window,
        binding: 1,
        chosen: None,
        band: None,
    });
    let mut frame = FrameSnapshot {
        revision: 0,
        project: Arc::new(state.history.project().clone()),
        timeline: 0,
        timeline_frame: 10,
        canvas: (128, 72),
        layers: vec![LayerSnapshot {
            clip: "c1".into(),
            origin: Origin {
                clip: "c1".into(),
                timeline_frame: 10,
                key_frame: 10,
                media: None,
            },
            shown_size: (128, 72),
            picture_to_canvas: [128.0, 0.0, 0.0, 72.0, 0.0, 0.0],
            opacity: 1.0,
            editable: true,
        }],
        comparison: json!({"mode":"off"}),
        ready: true,
        selection: Some(selection.clone()),
        overlay: None,
        frozen: 0,
        highlight: None,
        fusion: false,
    };
    frame.overlay = selection.overlay(&frame, None);
    let frame = Arc::new(frame);
    let registry = Registry::default();
    registry.activate(Monitor::Timeline, Some(1));
    let tag = registry
        .register(Monitor::Timeline, 1, frame.clone())
        .unwrap();
    let shown = Presentation {
        serial: 1,
        tag: Some(tag),
        map: columbina_view::Viewport::Fit.map((128, 72), (128, 72), 0),
    };
    registry.observe(Monitor::Timeline, 1, Some(shown));
    let pointer = columbina_view::Pointer {
        kind: PointerKind::Down,
        delta: 0,
        clicks: 1,
        middle_moved: false,
        button: Some(Button::Left),
        buttons: 1,
        inside: true,
        picture: Some((76.5, 36.5)),
        window: (76, 36),
        presentation: Some(shown),
        shift: false,
        ctrl: false,
        alt: false,
    };
    let event = registry.input(Monitor::Timeline, 7, 1, pointer, Some(frame.clone()));
    let pin = registry.claim(7, event).unwrap();
    state.window.owner = Some(7);
    state.window.selection = Some(selection);
    state
        .window_begin(7, 1, pin.clone(), registry.clone())
        .unwrap();
    (state, registry, pin, frame)
}
fn arm(state: &mut State) {
    let mut capture = state.window.active.take().unwrap();
    capture.gesture = Some(
        state
            .begin_owned_gesture(7, state.history.revision())
            .unwrap(),
    );
    state.window.active = Some(capture);
}

fn control_fixture(
    shape: WindowShape,
    at: (i32, i32),
) -> (State, Registry, Arc<Pin>, Arc<FrameSnapshot>) {
    control_window_fixture(Window::new("w1", shape), at)
}

fn control_window_fixture(
    window: Window,
    at: (i32, i32),
) -> (State, Registry, Arc<Pin>, Arc<FrameSnapshot>) {
    let (state, registry, pin, frame, begun) = try_control_window(window, at);
    begun.expect("visible control must capture");
    (state, registry, pin, frame)
}

#[allow(clippy::type_complexity, reason = "one fixture tuple")]
fn try_control_window(
    window: Window,
    at: (i32, i32),
) -> (
    State,
    Registry,
    Arc<Pin>,
    Arc<FrameSnapshot>,
    Result<Value, Fault>,
) {
    let (mut state, registry, old_pin, old_frame) = fixture();
    state.window_end("fixture");
    let mut frame = (*old_frame).clone();
    let mut project = (*frame.project).clone();
    let NodeKind::Corrector(c) = &mut project.timelines[0].tracks[0].clips[0].grade.nodes[0].kind
    else {
        panic!()
    };
    c.windows[0] = window;
    state.history = History::new(project.clone());
    frame.project = Arc::new(project);
    frame.overlay = state
        .window
        .selection
        .as_ref()
        .unwrap()
        .overlay(&frame, None);
    let frame = Arc::new(frame);
    let tag = registry
        .register(Monitor::Timeline, 1, frame.clone())
        .unwrap();
    let mut pointer = old_pin.down.pointer;
    pointer.window = at;
    pointer.presentation.as_mut().unwrap().tag = Some(tag);
    pointer.presentation.as_mut().unwrap().serial += 1;
    registry.observe(Monitor::Timeline, 1, pointer.presentation);
    let event = registry.input(Monitor::Timeline, 7, 1, pointer, Some(frame.clone()));
    let pin = registry.claim(7, event).unwrap();
    let begun = state.window_begin(7, 1, pin.clone(), registry.clone());
    (state, registry, pin, frame, begun)
}

/// Default transform on the 128x72 fixture: local (x, y) is device (64 + 72x, 36 + 72y).
fn drag(
    state: &mut State,
    registry: &Registry,
    pin: &Arc<Pin>,
    frame: &Arc<FrameSnapshot>,
    to: (i32, i32),
) {
    let mut pointer = pin.down.pointer;
    pointer.kind = PointerKind::Move;
    pointer.button = None;
    pointer.window = to;
    registry.input(Monitor::Timeline, 7, 1, pointer, Some(frame.clone()));
    let id = state.window.active.as_ref().unwrap().id;
    state.window_events(
        id,
        registry.events(pin),
        registry.current(Monitor::Timeline, 1),
    );
}

fn first_window(state: &State) -> Window {
    let selection = state.window.selection.as_ref().unwrap();
    selection
        .corrector(state.history.project())
        .unwrap()
        .windows[0]
        .clone()
}

fn path_points(window: &Window) -> Vec<columbina_model::grade::WindowPoint> {
    match &window.shape {
        WindowShape::Linear { points, .. } => points.to_vec(),
        WindowShape::Polygon { points, .. } => points.clone(),
        _ => panic!("not a path"),
    }
}

/// The real arming step: begin the owned gesture, then write any insertion as its first step.
fn arm_capture(state: &mut State) {
    let mut capture = state.window.active.take().unwrap();
    capture.gesture = Some(
        state
            .begin_owned_gesture(7, state.history.revision())
            .unwrap(),
    );
    state.window_write_insertion(&mut capture).unwrap();
    state.window.active = Some(capture);
}

#[test]
fn linear_corner_drag_uses_the_displayed_corner_but_writes_only_its_left_keys() {
    let mut window = Window::new("w1", WindowShape::linear());
    let WindowShape::Linear { points, .. } = &mut window.shape else {
        panic!()
    };
    // Displayed at -0.25 on frame 10 (E), its left key holds -0.5 (L).
    points[0].x = Param::Keys(vec![
        Key {
            frame: 0,
            value: -0.5,
            interp: Interp::Linear,
        },
        Key {
            frame: 20,
            value: 0.0,
            interp: Interp::Hold,
        },
    ]);
    let before = window.clone();
    let (mut state, registry, pin, frame) = control_window_fixture(window, (46, 18));
    assert_eq!(
        state.window.active.as_ref().unwrap().movement.keys(),
        json!({"points":[{"point":0,"x":0,"y":null}],"clock":"source","frame":10})
    );
    arm(&mut state);
    drag(&mut state, &registry, &pin, &frame, (64, 27));
    let points = path_points(&first_window(&state));
    let Param::Keys(keys) = &points[0].x else {
        panic!()
    };
    assert!((keys[0].value - -0.25).abs() < 1e-12, "L + 18 px");
    assert_eq!(keys[1].value, 0.0);
    assert!((points[0].y.value_at(10.0) - -0.125).abs() < 1e-12);
    assert_eq!(points[1..], path_points(&before)[1..]);
    assert_eq!(first_window(&state).transform, before.transform);
}

#[test]
fn linear_side_drag_moves_both_ends_and_its_softness_handle_edits_only_that_side() {
    let (mut state, registry, pin, frame) =
        control_window_fixture(Window::new("w1", WindowShape::linear()), (64, 18));
    arm(&mut state);
    drag(&mut state, &registry, &pin, &frame, (64, 36));
    let points = path_points(&first_window(&state));
    for (point, want) in points
        .iter()
        .zip([(-0.25, 0.0), (0.25, 0.0), (0.25, 0.25), (-0.25, 0.25)])
    {
        assert!((point.x.value_at(0.0) - want.0).abs() < 1e-12, "{point:?}");
        assert!((point.y.value_at(0.0) - want.1).abs() < 1e-12, "{point:?}");
    }
    // Soft 1's handle is 3/4 along the top side, just outside it; up by 9 px is +50.
    let (mut state, registry, pin, frame) =
        control_window_fixture(Window::new("w1", WindowShape::linear()), (72, 17));
    assert_eq!(
        state.window.active.as_ref().unwrap().movement.keys(),
        json!({"softness":null,"edge":0,"clock":"source","frame":10})
    );
    arm(&mut state);
    drag(&mut state, &registry, &pin, &frame, (72, 8));
    let WindowShape::Linear { softness, points } = first_window(&state).shape else {
        panic!()
    };
    assert!((softness[0].value_at(0.0) - 51.0).abs() < 1e-9);
    assert_eq!(
        softness[1..],
        [Param::Fixed(1.0), Param::Fixed(1.0), Param::Fixed(1.0)]
    );
    let WindowShape::Linear {
        points: initial, ..
    } = WindowShape::linear()
    else {
        unreachable!()
    };
    assert_eq!(points, initial);
}

#[test]
fn polygon_point_drag_and_body_move_keep_the_point_order() {
    let (mut state, registry, pin, frame) =
        control_window_fixture(Window::new("w1", WindowShape::polygon()), (82, 54));
    arm(&mut state);
    drag(&mut state, &registry, &pin, &frame, (91, 63));
    let points = path_points(&first_window(&state));
    assert_eq!(
        (points[2].x.value_at(0.0), points[2].y.value_at(0.0)),
        (0.375, 0.375)
    );
    assert_eq!(points.len(), 4);
    let (mut state, registry, pin, frame) =
        control_window_fixture(Window::new("w1", WindowShape::polygon()), (60, 40));
    arm(&mut state);
    drag(&mut state, &registry, &pin, &frame, (76, 40));
    let window = first_window(&state);
    assert_eq!(window.transform.pan, Param::Fixed(62.5));
    assert_eq!(path_points(&window).len(), 4);
}

#[test]
fn polygon_edge_insertion_and_its_drag_are_one_undo_step_and_keep_the_down_original() {
    let (mut state, registry, pin, frame) =
        control_window_fixture(Window::new("w1", WindowShape::polygon()), (64, 18));
    let before = state.history.project().clone();
    let original = state
        .window
        .active
        .as_ref()
        .unwrap()
        .movement
        .original
        .clone();
    assert_eq!(
        state.window.active.as_ref().unwrap().movement.keys(),
        json!({"points":[{"point":1,"x":null,"y":null}],"inserted":1,"clock":"source","frame":10})
    );
    arm_capture(&mut state);
    let points = path_points(&first_window(&state));
    assert_eq!(points.len(), 5);
    assert!(
        (points[1].x.value_at(0.0) - 0.5 / 72.0).abs() < 1e-12,
        "the clicked share"
    );
    assert!(matches!(points[1].y, Param::Fixed(y) if (y - -0.25).abs() < 1e-12));
    drag(&mut state, &registry, &pin, &frame, (64, 27));
    let points = path_points(&first_window(&state));
    assert!(matches!(points[1].y, Param::Fixed(y) if (y - -0.125).abs() < 1e-12));
    assert_eq!(
        (points[0].clone(), points[2].clone()),
        (
            path_points(&original[0])[0].clone(),
            path_points(&original[0])[1].clone()
        )
    );
    assert_eq!(path_points(&original[0]).len(), 4, "Esc restores this list");
    state.window_end("up");
    state.history.undo("desktop").unwrap().unwrap();
    assert_eq!(state.history.project(), &before);
    assert!(!state.history.can_undo());
}

#[test]
fn esc_after_an_edge_insertion_and_its_drag_restores_four_points_without_an_undo_step() {
    let (mut state, registry, pin, frame) =
        control_window_fixture(Window::new("w1", WindowShape::polygon()), (64, 18));
    let before = state.history.project().clone();
    arm_capture(&mut state);
    drag(&mut state, &registry, &pin, &frame, (64, 27));
    assert_eq!(path_points(&first_window(&state)).len(), 5);
    assert!(state.history.can_undo());
    // The same route as `window.input {kind:"cancel"}` once its capture is checked.
    let capture = state.window.active.take().unwrap();
    let ended = state.window_cancel(7, capture);
    assert_eq!(
        (ended["phase"].as_str(), ended["reason"].as_str()),
        (Some("ended"), Some("cancel"))
    );
    assert_eq!(path_points(&first_window(&state)).len(), 4);
    assert_eq!(state.history.project(), &before);
    assert!(
        !state.history.can_undo(),
        "the cancelled gesture leaves no step"
    );
    assert!(state.history.gesture().is_none());
    assert!(!pin.live.load(Ordering::Acquire));
}

#[test]
fn a_released_edge_click_before_arming_inserts_once_and_an_unreleased_one_writes_nothing() {
    for released in [true, false] {
        let (mut state, registry, pin, frame) =
            control_window_fixture(Window::new("w1", WindowShape::polygon()), (64, 18));
        if released {
            let mut pointer = pin.down.pointer;
            pointer.kind = PointerKind::Up;
            pointer.buttons = 0;
            registry.input(Monitor::Timeline, 7, 1, pointer, Some(frame.clone()));
        }
        let capture = state.window.active.take().unwrap();
        let ended = state.window_finish_unarmed(capture, "native_end");
        assert_eq!(ended["phase"], if released { "ended" } else { "refused" });
        assert_eq!(state.history.revision(), u64::from(released));
        assert_eq!(
            path_points(&first_window(&state)).len(),
            if released { 5 } else { 4 }
        );
    }
}

#[test]
fn an_animated_edge_that_cannot_keep_its_shape_refuses_the_whole_insertion_at_down() {
    let mut window = Window::new("w1", WindowShape::polygon());
    let WindowShape::Polygon { points, .. } = &mut window.shape else {
        panic!()
    };
    let keys = |interp| {
        Param::Keys(vec![
            Key {
                frame: 0,
                value: -0.25,
                interp,
            },
            Key {
                frame: 20,
                value: 0.0,
                interp: Interp::Linear,
            },
        ])
    };
    points[0].y = keys(Interp::Hold);
    points[1].y = keys(Interp::Linear);
    let (state, _, _, _, begun) = try_control_window(window, (64, 22));
    let fault = begun.unwrap_err();
    assert_eq!(
        fault.data,
        Some(
            json!({"reason":"insert","topology":{"why":"jump","axis":"y","points":[0,1],
            "frames":[0,20],"clock":"source"}})
        )
    );
    assert!(state.window.active.is_none());
    assert_eq!(state.history.revision(), 0);
}

fn middle_tap(
    registry: &Registry,
    pin: &Arc<Pin>,
    frame: &Arc<FrameSnapshot>,
    at: (i32, i32),
    moved: bool,
) -> Tap {
    let mut pointer = pin.down.pointer;
    pointer.kind = PointerKind::Down;
    pointer.button = Some(Button::Middle);
    pointer.buttons = 4;
    pointer.window = at;
    let event = registry.input(Monitor::Timeline, 7, 1, pointer, Some(frame.clone()));
    pointer.kind = PointerKind::Up;
    pointer.buttons = 0;
    pointer.middle_moved = moved;
    registry.input(Monitor::Timeline, 7, 1, pointer, Some(frame.clone()));
    assert!(
        registry.claim_tap(8, event).is_none(),
        "only its owner claims it"
    );
    registry.claim_tap(7, event).unwrap()
}

#[test]
fn a_waiting_middle_tap_keeps_no_project_and_a_pan_end_keeps_no_controls() {
    let (mut state, registry, pin, frame) =
        control_window_fixture(Window::new("w1", WindowShape::polygon()), (60, 40));
    state.window_end("fixture");
    let held = Arc::strong_count(&frame.project);
    let mut pointer = pin.down.pointer;
    pointer.kind = PointerKind::Down;
    pointer.button = Some(Button::Middle);
    pointer.buttons = 4;
    pointer.window = (82, 54);
    let event = registry.input(Monitor::Timeline, 7, 1, pointer, Some(frame.clone()));
    assert_eq!(
        Arc::strong_count(&frame.project),
        held,
        "a tap waiting for its release or claim holds no Project"
    );
    pointer.kind = PointerKind::Up;
    pointer.buttons = 0;
    pointer.middle_moved = true;
    registry.input(Monitor::Timeline, 7, 1, pointer, Some(frame.clone()));
    assert_eq!(Arc::strong_count(&frame.project), held);
    let tap = registry.claim_tap(7, event).unwrap();
    assert!(tap.down.frame.is_none() && tap.up.as_ref().unwrap().frame.is_none());
    assert!(
        tap.shown.is_none(),
        "a pan's end lets the down's controls go"
    );
    assert_eq!(
        state.window_delete(7, 1, &tap).unwrap_err().data,
        Some(json!({"reason":"moved"}))
    );
}

#[test]
fn a_middle_tap_deletes_a_polygon_point_for_the_whole_animation_in_one_step() {
    let mut window = Window::new("w1", WindowShape::polygon());
    let WindowShape::Polygon { points, .. } = &mut window.shape else {
        panic!()
    };
    points[3].x = Param::Keys(vec![Key {
        frame: 4,
        value: -0.25,
        interp: Interp::Linear,
    }]);
    let kept = [points[0].clone(), points[1].clone(), points[3].clone()];
    let (mut state, registry, pin, frame) = control_window_fixture(window, (60, 40));
    state.window_end("fixture");
    let before = state.history.project().clone();
    // A moved middle gesture is a pan, not a delete.
    let moved = middle_tap(&registry, &pin, &frame, (82, 54), true);
    assert_eq!(
        state.window_delete(7, 1, &moved).unwrap_err().data,
        Some(json!({"reason":"moved"}))
    );
    let tap = middle_tap(&registry, &pin, &frame, (82, 54), false);
    assert!(
        registry.claim_tap(7, tap.down.sequence).is_none(),
        "claimed once"
    );
    let deleted = state.window_delete(7, 1, &tap).unwrap();
    assert_eq!(deleted["point"], 2);
    assert_eq!(deleted["selected"], "w1");
    assert_eq!(path_points(&first_window(&state)), kept);
    state.history.undo("desktop").unwrap().unwrap();
    assert_eq!(state.history.project(), &before);
}

#[test]
fn a_middle_tap_keeps_three_polygon_points_and_never_deletes_linear_corners() {
    use columbina_model::grade::WindowPoint;
    let triangle = Window::new(
        "w1",
        WindowShape::Polygon {
            points: vec![
                WindowPoint::new(-0.25, 0.25),
                WindowPoint::new(0.0, -0.25),
                WindowPoint::new(0.25, 0.25),
            ],
            inside: 0.0.into(),
            outside: 0.0.into(),
        },
    );
    let (mut state, registry, pin, frame) = control_window_fixture(triangle, (64, 40));
    state.window_end("fixture");
    let tap = middle_tap(&registry, &pin, &frame, (82, 54), false);
    assert_eq!(
        state.window_delete(7, 1, &tap).unwrap_err().data,
        Some(json!({"reason":"delete","topology":{"why":"minimum","least":3,"shape":"polygon"}}))
    );
    assert_eq!(state.history.revision(), 0);
    let (mut state, registry, pin, frame) =
        control_window_fixture(Window::new("w1", WindowShape::linear()), (60, 40));
    state.window_end("fixture");
    let tap = middle_tap(&registry, &pin, &frame, (82, 54), false);
    assert_eq!(
        state.window_delete(7, 1, &tap).unwrap_err().data,
        Some(json!({"reason":"hit"}))
    );
    assert_eq!(state.history.revision(), 0);
}

#[test]
fn transform_corner_scales_without_moving_the_centre() {
    let (mut state, registry, pin, frame) = control_fixture(WindowShape::circular(), (82, 54));
    arm(&mut state);
    let mut pointer = pin.down.pointer;
    pointer.kind = PointerKind::Move;
    pointer.window = (91, 63);
    registry.input(Monitor::Timeline, 7, 1, pointer, Some(frame));
    let id = state.window.active.as_ref().unwrap().id;
    state.window_events(
        id,
        registry.events(&pin),
        registry.current(Monitor::Timeline, 1),
    );
    let selection = state.window.selection.as_ref().unwrap();
    let changed = &selection
        .corrector(state.history.project())
        .unwrap()
        .windows[0];
    assert_eq!(changed.transform.size, Param::Fixed(75.0));
    assert_eq!(changed.transform.pan, Param::Fixed(50.0));
    assert_eq!(changed.transform.tilt, Param::Fixed(50.0));
}

#[test]
fn transform_gradient_centre_is_a_native_move_control() {
    let (mut state, registry, pin, frame) = control_fixture(WindowShape::gradient(), (64, 36));
    arm(&mut state);
    let mut pointer = pin.down.pointer;
    pointer.kind = PointerKind::Move;
    pointer.window = (80, 36);
    registry.input(Monitor::Timeline, 7, 1, pointer, Some(frame));
    let id = state.window.active.as_ref().unwrap().id;
    state.window_events(
        id,
        registry.events(&pin),
        registry.current(Monitor::Timeline, 1),
    );
    let selection = state.window.selection.as_ref().unwrap();
    let changed = &selection
        .corrector(state.history.project())
        .unwrap()
        .windows[0];
    assert_eq!(changed.transform.pan, Param::Fixed(62.5));
}

#[test]
fn transform_rotation_retains_a_whole_observed_turn_before_move_coalescing() {
    let (mut state, registry, pin, frame) = control_fixture(WindowShape::circular(), (64, 22));
    arm(&mut state);
    for point in [
        (74, 26),
        (78, 36),
        (74, 46),
        (64, 50),
        (54, 46),
        (50, 36),
        (54, 26),
        (64, 22),
    ] {
        let mut pointer = pin.down.pointer;
        pointer.kind = PointerKind::Move;
        pointer.window = point;
        registry.input(Monitor::Timeline, 7, 1, pointer, Some(frame.clone()));
    }
    assert_eq!(registry.events(&pin).len(), 1);
    let id = state.window.active.as_ref().unwrap().id;
    state.window_events(
        id,
        registry.events(&pin),
        registry.current(Monitor::Timeline, 1),
    );
    let selection = state.window.selection.as_ref().unwrap();
    let changed = &selection
        .corrector(state.history.project())
        .unwrap()
        .windows[0];
    assert!((changed.transform.rotate.value_at(10.0) - 360.0).abs() < 1e-9);
    assert_eq!(changed.transform.pan, Param::Fixed(50.0));
}

#[test]
fn transform_rotation_rebases_after_an_observed_centre_without_inventing_a_turn() {
    let mut window = Window::new("w1", WindowShape::circular());
    window.transform.pan = Param::Fixed(50.0 + 100.0 * 0.5 / 128.0);
    window.transform.tilt = Param::Fixed(50.0 - 100.0 * 0.5 / 72.0);
    let (mut state, registry, pin, frame) = control_window_fixture(window, (64, 22));
    arm(&mut state);
    for point in [(78, 36), (64, 36), (50, 36), (64, 22)] {
        let mut pointer = pin.down.pointer;
        pointer.kind = PointerKind::Move;
        pointer.window = point;
        registry.input(Monitor::Timeline, 7, 1, pointer, Some(frame.clone()));
    }
    let id = state.window.active.as_ref().unwrap().id;
    state.window_events(
        id,
        registry.events(&pin),
        registry.current(Monitor::Timeline, 1),
    );
    let selection = state.window.selection.as_ref().unwrap();
    let changed = &selection
        .corrector(state.history.project())
        .unwrap()
        .windows[0];
    assert!((changed.transform.rotate.value_at(10.0) - 180.0).abs() < 1e-9);
}

#[test]
fn transform_rotation_removes_picture_placement_but_keeps_window_aspect_out_of_angle_basis() {
    let (state, _, _, frame) = control_fixture(WindowShape::circular(), (64, 22));
    let selection = state.window.selection.as_ref().unwrap();
    let mut frame = (*frame).clone();
    frame.layers[0].shown_size = (640, 360);
    frame.layers[0].picture_to_canvas = [0.0, -360.0, 640.0, 0.0, 900.0, 0.0];
    let NodeKind::Corrector(c) = &mut Arc::make_mut(&mut frame.project).timelines[0].tracks[0]
        .clips[0]
        .grade
        .nodes[0]
        .kind
    else {
        panic!()
    };
    c.windows[0].transform.aspect = Param::Fixed(100.0);
    let basis = crate::window::RotationBasis::of(selection, &frame, "w1").unwrap();
    assert!(basis.angle((720.0, 356.0)).unwrap().abs() < 1e-9);
    assert!((basis.angle((684.0, 320.0)).unwrap().to_degrees() - 90.0).abs() < 1e-9);
    assert!((basis.angle((684.0, 356.0)).unwrap().to_degrees() - 45.0).abs() < 1e-9);
    assert!(basis.angle((720.0, 320.0)).is_none());
}

#[test]
#[allow(
    clippy::cast_possible_truncation,
    reason = "test pixels near the centre"
)]
fn transform_rotation_ignores_pointers_within_one_device_pixel_of_the_centre() {
    let (state, _, pin, frame) = control_fixture(WindowShape::circular(), (64, 22));
    let selection = state.window.selection.as_ref().unwrap();
    let map = pin.down.pointer.presentation.unwrap().map.unwrap();
    let basis = crate::window::RotationBasis::of(selection, &frame, "w1").unwrap();
    let shape = &frame.overlay.as_ref().unwrap().shapes[0];
    let (cx, cy) = map.to_device((shape.to_canvas.tx, shape.to_canvas.ty));
    let (px, py) = ((cx - 0.5).round() as i32, (cy - 0.5).round() as i32);
    let mut inside = 0;
    for dy in -2..=2 {
        for dx in -2..=2 {
            let point = (px + dx, py + dy);
            let distance = (f64::from(point.0) + 0.5 - cx).hypot(f64::from(point.1) + 0.5 - cy);
            if distance > 1e-9 && distance < 1.0 {
                inside += 1;
            }
            assert_eq!(
                basis.angle_at(map, point).is_none(),
                distance < 1.0,
                "{point:?} at {distance} px from the centre"
            );
        }
    }
    assert!(
        inside > 0,
        "the fixture centre must fall between pixel centres"
    );
    // Away from the dead zone the device reading is the canvas angle.
    let far = (px + 6, py);
    assert_eq!(basis.angle_at(map, far), basis.angle(map.on_picture(far).0));
}

#[test]
fn transform_rotation_keeps_pre_arm_frame_advancement_but_rejects_hidden_post_arm_changes() {
    for pre_arm in [false, true] {
        let (mut state, registry, pin, frame) = control_fixture(WindowShape::circular(), (64, 22));
        let mut intervening = (*frame).clone();
        intervening.timeline_frame += 1;
        intervening.layers[0].origin.key_frame += 1;
        let mut pointer = pin.down.pointer;
        pointer.kind = PointerKind::Move;
        pointer.window = (74, 26);
        let event = registry.input(
            Monitor::Timeline,
            7,
            1,
            pointer,
            Some(Arc::new(intervening)),
        );
        if pre_arm {
            state.window.active.as_mut().unwrap().arming_through = event;
        }
        arm(&mut state);
        pointer.window = (78, 36);
        registry.input(Monitor::Timeline, 7, 1, pointer, Some(frame));
        let id = state.window.active.as_ref().unwrap().id;
        state.window_events(
            id,
            registry.events(&pin),
            registry.current(Monitor::Timeline, 1),
        );
        assert_eq!(state.window.active.is_some(), pre_arm);
        assert_eq!(state.history.revision() > 0, pre_arm);
    }
}

#[test]
fn transform_rotation_does_not_hide_an_intermediate_changed_map() {
    let (mut state, registry, pin, frame) = control_fixture(WindowShape::circular(), (64, 22));
    arm(&mut state);
    let mut pointer = pin.down.pointer;
    pointer.kind = PointerKind::Move;
    pointer.window = (74, 26);
    pointer.presentation.as_mut().unwrap().map =
        columbina_view::Viewport::Fit.map((256, 144), (128, 72), 0);
    registry.input(Monitor::Timeline, 7, 1, pointer, Some(frame.clone()));
    pointer.presentation = pin.down.pointer.presentation;
    pointer.window = (78, 36);
    registry.input(Monitor::Timeline, 7, 1, pointer, Some(frame));
    let id = state.window.active.as_ref().unwrap().id;
    state.window_events(
        id,
        registry.events(&pin),
        registry.current(Monitor::Timeline, 1),
    );
    assert!(state.window.active.is_none());
    assert_eq!(state.history.revision(), 0);
}

#[test]
fn transform_controls_use_evaluated_geometry_but_write_only_their_left_key() {
    use columbina_view::window_overlay::Control;
    let (state, _, _, frame) = fixture();
    let selection = state.window.selection.as_ref().unwrap();
    let mut frame = (*frame).clone();
    let NodeKind::Corrector(c) = &mut Arc::make_mut(&mut frame.project).timelines[0].tracks[0]
        .clips[0]
        .grade
        .nodes[0]
        .kind
    else {
        panic!()
    };
    c.windows[0] = Window::new("w1", WindowShape::circular());
    c.windows[0].transform.size = Param::Keys(vec![
        Key {
            frame: 0,
            value: 0.0,
            interp: Interp::Linear,
        },
        Key {
            frame: 20,
            value: 100.0,
            interp: Interp::Hold,
        },
    ]);
    let size = Movement::capture(
        selection,
        &frame,
        "w1",
        Control::Size(2),
        (82.0, 54.0),
        [1.0; 2],
        Modifiers::default(),
    )
    .unwrap();
    let changed = size.at((91.0, 63.0)).unwrap();
    assert_eq!(size.keys(), json!({"size":0,"clock":"source","frame":10}));
    let Param::Keys(keys) = &changed[0].transform.size else {
        panic!()
    };
    assert_eq!(keys[0].value, 25.0, "L=0, E=50 and delta=25");
    assert_eq!(keys[1].value, 100.0);
    assert_eq!(changed[0].transform.pan, size.original[0].transform.pan);
    assert_eq!(changed[1], size.original[1]);
    let NodeKind::Corrector(c) = &mut Arc::make_mut(&mut frame.project).timelines[0].tracks[0]
        .clips[0]
        .grade
        .nodes[0]
        .kind
    else {
        panic!()
    };
    c.windows[0].transform.aspect = Param::Keys(vec![
        Key {
            frame: 2,
            value: 10.0,
            interp: Interp::Linear,
        },
        Key {
            frame: 18,
            value: 90.0,
            interp: Interp::Hold,
        },
    ]);
    let aspect = Movement::capture(
        selection,
        &frame,
        "w1",
        Control::Aspect(1),
        (82.0, 36.0),
        [1.0; 2],
        Modifiers::default(),
    )
    .unwrap();
    let changed = aspect.at((100.0, 36.0)).unwrap();
    assert_eq!(aspect.keys()["aspect"], 2);
    assert_eq!(changed[0].transform.aspect.value_at(2.0), 60.0);
    assert_eq!(changed[0].transform.size, aspect.original[0].transform.size);
    assert_eq!(
        aspect.at((-1000.0, 36.0)).unwrap()[0]
            .transform
            .aspect
            .value_at(2.0),
        0.0
    );
}

#[test]
fn transform_softness_controls_restore_zero_and_clamp_without_touching_transforms() {
    use columbina_view::window_overlay::Control;
    for shape in [WindowShape::circular(), WindowShape::gradient()] {
        let (state, _, _, frame) = control_fixture(shape.clone(), (64, 36));
        let selection = state.window.selection.as_ref().unwrap();
        for side in [-1, 1] {
            if matches!(shape, WindowShape::Circular { .. }) && side == -1 {
                continue;
            }
            let movement = Movement::capture(
                selection,
                &frame,
                "w1",
                Control::Softness(side),
                (64.0, 36.0),
                [1.0; 2],
                Modifiers::default(),
            )
            .unwrap();
            let changed = movement.at((64.0, 36.0 + f64::from(side) * 18.0)).unwrap();
            let (WindowShape::Circular { softness: soft }
            | WindowShape::Gradient { softness: soft }) = &changed[0].shape
            else {
                panic!()
            };
            let expected = if matches!(shape, WindowShape::Circular { .. }) {
                51.0
            } else {
                100.0
            };
            assert!((soft.value_at(10.0) - expected).abs() < 1e-9);
            assert_eq!(changed[0].transform, movement.original[0].transform);
        }
    }
}
/// Author decision 2026-11-02: a Circular/Gradient softness ring is drawn but never hit, like
/// the Linear/Polygon guides; the explicit softness handles still drag the softness.
#[test]
fn softness_rings_never_capture_but_their_handles_still_drag_softness() {
    let soft = |shape: WindowShape| {
        let mut window = Window::new("w1", shape);
        let (WindowShape::Circular { softness } | WindowShape::Gradient { softness }) =
            &mut window.shape
        else {
            panic!()
        };
        // 40 is .1 image heights each side: 7.2 device pixels on the 128x72 fixture.
        *softness = Param::Fixed(40.0);
        window
    };
    // Circular: radius 18 px, outer ring at 25.2 px; its handle at 30° on that ring.
    let (_, _, _, _, ring) = try_control_window(soft(WindowShape::circular()), (64 + 25, 36));
    assert_eq!(ring.unwrap_err().data, Some(json!({"reason":"hit"})));
    let (mut state, registry, pin, frame) =
        control_window_fixture(soft(WindowShape::circular()), (85, 48));
    assert_eq!(
        state.window.active.as_ref().unwrap().movement.control,
        columbina_view::window_overlay::Control::Softness(1)
    );
    arm_capture(&mut state);
    drag(&mut state, &registry, &pin, &frame, (89, 48));
    let WindowShape::Circular { softness } = &first_window(&state).shape else {
        panic!()
    };
    assert!(softness.value_at(10.0) > 40.0, "{softness:?}");
    // Gradient: the line 7.2 px on the dark side, away from the handle at (82, 43).
    let (_, _, _, _, ring) = try_control_window(soft(WindowShape::gradient()), (30, 43));
    assert_eq!(ring.unwrap_err().data, Some(json!({"reason":"hit"})));
    let (state, _, _, _) = control_window_fixture(soft(WindowShape::gradient()), (82, 43));
    assert_eq!(
        state.window.active.as_ref().unwrap().movement.control,
        columbina_view::window_overlay::Control::Softness(1)
    );
}

fn movement(
    state: &mut State,
    registry: &Registry,
    pin: &Arc<Pin>,
    frame: &Arc<FrameSnapshot>,
    x: i32,
) {
    let mut p = pin.down.pointer;
    p.kind = PointerKind::Move;
    p.button = None;
    p.window.0 = x;
    registry.input(Monitor::Timeline, 7, 1, p, Some(frame.clone()));
    let events = registry.events(pin);
    state.window_events(1, events, registry.current(Monitor::Timeline, 1));
}

#[test]
fn captured_whole_list_moves_survive_ten_second_idle_and_undo_once() {
    let (mut state, registry, pin, frame) = fixture();
    let before = state.history.project().clone();
    arm(&mut state);
    movement(&mut state, &registry, &pin, &frame, 84);
    assert_eq!(state.history.revision(), 1);
    state.end_idle_gesture(Instant::now() + GESTURE_IDLE + Duration::from_secs(2));
    assert!(
        state.history.gesture().is_some(),
        "a live native owner keeps one gesture"
    );
    movement(&mut state, &registry, &pin, &frame, 92);
    assert_eq!(
        state.history.revision(),
        2,
        "own revision redraw must keep the capture"
    );
    state.window_end("up");
    state.history.undo("desktop").unwrap().unwrap();
    assert_eq!(state.history.project(), &before);
    assert!(!state.history.can_undo());
}

#[test]
fn an_unwritten_click_and_arming_cancel_never_start_history() {
    for armed in [false, true] {
        let (mut state, _, pin, _) = fixture();
        if armed {
            arm(&mut state);
        }
        state.window_end("up");
        assert_eq!(state.history.revision(), 0);
        assert!(!state.history.can_undo());
        assert!(!pin.live.load(Ordering::Acquire));
    }
}

#[test]
fn successful_external_edit_ends_capture_but_failed_edit_does_not() {
    let (mut state, registry, pin, frame) = fixture();
    arm(&mut state);
    movement(&mut state, &registry, &pin, &frame, 84);
    let error = state.apply_worked_out_locked(Kind::Agent, None, None, Instant::now(), |_| {
        Ok(vec![json!({"op":"remove_bin","id":"missing"})])
    });
    assert!(error.is_err());
    assert!(state.window.active.is_some());
    state
        .apply_worked_out_locked(Kind::Agent, None, None, Instant::now(), |_| {
            Ok(vec![
                json!({"op":"set_track","timeline":"t1","id":"V1","name":"other"}),
            ])
        })
        .unwrap();
    assert!(state.window.active.is_none());
    assert!(!pin.live.load(Ordering::Acquire));
}

fn binding_peer(
    state: &mut State,
    owner: u64,
) -> (crate::queue::Receiver, tokio::sync::watch::Receiver<bool>) {
    let (sender, receiver) = crate::queue::queue(crate::queue::Limits {
        messages: 100,
        bytes: 1 << 20,
    });
    let (stop, stopped) = tokio::sync::watch::channel(false);
    state.peers.insert(
        owner,
        crate::session::Peer {
            kind: Kind::Desktop,
            queue: sender,
            stop,
        },
    );
    (receiver, stopped)
}

fn window_notifications(receiver: &mut crate::queue::Receiver) -> Vec<Value> {
    let mut notes = Vec::new();
    while let Some(line) = receiver.try_recv() {
        let note: Value = serde_json::from_slice(line.as_ref()).unwrap();
        if note["method"]
            .as_str()
            .is_some_and(|method| method.starts_with("window."))
        {
            notes.push(note);
        }
    }
    notes
}

fn disconnect_window_grade(state: &mut State) {
    state.apply_worked_out_locked(Kind::Agent, None, None, Instant::now(), |_| {
        Ok(vec![json!({"op":"disconnect","grade":{"level":"clip","timeline":"t1","clip":"c1"},"to":"n1","kind":"rgb","input":0})])
    }).unwrap();
}

#[test]
fn invalid_project_retires_idle_binding_once_for_its_owner() {
    let (mut state, _, _, _) = fixture();
    state.window_end("up");
    let binding = state.window.selection.as_ref().unwrap().binding;
    let sequence = state.window.sequence;
    let (mut owner, _) = binding_peer(&mut state, 7);
    let (mut observer, _) = binding_peer(&mut state, 8);
    disconnect_window_grade(&mut state);
    let revision = state.history.revision();
    assert!(state.window.owner.is_none());
    assert!(state.window.selection.is_none());
    // A desktop refetch can see only the repaired final graph. Retirement must still be
    // explicit even though neither project snapshot exposed the intermediate invalid grade.
    state.apply_worked_out_locked(Kind::Agent, None, None, Instant::now(), |_| {
        Ok(vec![json!({"op":"connect","grade":{"level":"clip","timeline":"t1","clip":"c1"},"link":{"from":"source","to":"n1","kind":"rgb","input":0}})])
    }).unwrap();
    assert!(state.history.project().timelines[0].tracks[0].clips[0]
        .grade
        .is_complete());
    let notes = window_notifications(&mut owner);
    assert_eq!(
        notes.len(),
        1,
        "an idle invalidated binding must notify its owner once"
    );
    assert_eq!(notes[0]["method"], "window.unbound");
    assert_eq!(
        notes[0]["params"],
        json!({"binding":binding,"sequence":state.window.sequence,"revision":revision,"reason":"project_changed"})
    );
    assert!(state.window.sequence > sequence);
    assert_eq!(window_notifications(&mut observer), Vec::<Value>::new());
}

#[test]
fn binding_retirement_follows_the_active_capture_end() {
    let (mut state, registry, pin, frame) = fixture();
    arm(&mut state);
    movement(&mut state, &registry, &pin, &frame, 84);
    let (mut owner, _) = binding_peer(&mut state, 7);
    disconnect_window_grade(&mut state);
    let notes = window_notifications(&mut owner);
    assert_eq!(notes.len(), 2);
    assert_eq!(notes[0]["method"], "window.state");
    assert_eq!(notes[0]["params"]["phase"], "ended");
    assert_eq!(notes[1]["method"], "window.unbound");
    assert_eq!(notes[0]["params"]["binding"], notes[1]["params"]["binding"]);
    assert!(notes[0]["params"]["sequence"].as_u64() < notes[1]["params"]["sequence"].as_u64());
    assert_eq!(notes[1]["params"]["revision"], state.history.revision());
    assert!(!pin.live.load(Ordering::Acquire));
    assert!(state.window.active.is_none());
    assert!(state.history.gesture().is_none());
}

#[test]
fn valid_edits_and_explicit_unbind_do_not_report_binding_retirement() {
    let (mut state, registry, pin, frame) = fixture();
    let binding = state.window.selection.as_ref().unwrap().binding;
    let (mut owner, _) = binding_peer(&mut state, 7);
    arm(&mut state);
    movement(&mut state, &registry, &pin, &frame, 84);
    assert!(state
        .apply_worked_out_locked(Kind::Agent, None, None, Instant::now(), |_| {
            Ok(vec![json!({"op":"remove_bin","id":"missing"})])
        })
        .is_err());
    state
        .apply_worked_out_locked(Kind::Agent, None, None, Instant::now(), |_| {
            Ok(vec![
                json!({"op":"set_track","timeline":"t1","id":"V1","name":"still valid"}),
            ])
        })
        .unwrap();
    assert_eq!(state.window.selection.as_ref().unwrap().binding, binding);
    assert_eq!(state.window.owner, Some(7));
    state.window_unbind("unbind");
    assert!(window_notifications(&mut owner)
        .iter()
        .all(|note| note["method"] != "window.unbound"));
}

#[test]
fn binding_retirement_queue_failure_stops_only_the_owner() {
    let (mut state, _, _, _) = fixture();
    state.window_end("up");
    let (_owner, stopped) = binding_peer(&mut state, 7);
    let (mut observer, observer_stopped) = binding_peer(&mut state, 8);
    for _ in 0..100 {
        state.peers[&7]
            .queue
            .push(rpc::notification_line("test", &Value::Null))
            .unwrap();
    }
    // Change history without publishing general project notifications, so only the
    // retirement notification can cause the stop signal checked below.
    state
        .history
        .apply_unrecorded(
            vec![Op::Disconnect {
                grade: Target::Clip {
                    timeline: "t1".into(),
                    clip: "c1".into(),
                },
                to: "n1".into(),
                kind: columbina_model::grade::Port::Rgb,
                input: 0,
            }],
            "engine",
        )
        .unwrap();
    assert!(!*stopped.borrow());
    state.window_project_changed();
    assert!(*stopped.borrow());
    assert!(!*observer_stopped.borrow());
    assert_eq!(window_notifications(&mut observer), Vec::<Value>::new());
    assert!(state.window.selection.is_none());
}

#[test]
fn the_streams_of_media_read_by_the_engine_leave_a_window_drag_going() {
    let (mut state, registry, pin, frame) = fixture();
    arm(&mut state);
    movement(&mut state, &registry, &pin, &frame, 84);
    let gesture = state
        .history
        .gesture()
        .map(|(key, base)| (key.to_owned(), base));
    let applied = state
        .history
        .apply_unrecorded(
            vec![Op::SetMediaStreams {
                id: "m1".into(),
                layout: Some("stereo".into()),
                streams: Some(vec![]),
            }],
            "engine",
        )
        .unwrap();
    state.publish(&applied);
    // Neither picture nor sound changed (T3c): the drag goes on, as does its gesture.
    assert!(state.window.active.is_some());
    assert_eq!(
        state
            .history
            .gesture()
            .map(|(key, base)| (key.to_owned(), base)),
        gesture
    );
}

#[test]
fn unrecorded_external_revision_flushes_only_the_captures_old_key() {
    let (mut state, registry, pin, frame) = fixture();
    arm(&mut state);
    movement(&mut state, &registry, &pin, &frame, 84);
    let applied = state
        .history
        .apply_unrecorded(
            vec![Op::SetTrack {
                timeline: "t1".into(),
                id: "V1".into(),
                name: Some("proxy-like".into()),
                color: None,
                enabled: None,
                locked: None,
                language: None,
                mute: None,
                solo: None,
                solo_safe: None,
            }],
            "engine",
        )
        .unwrap();
    state.publish(&applied);
    assert!(state.window.active.is_none());
    assert!(state.history.gesture().is_none());
    assert!(state.held.is_none());
}

#[test]
fn native_wheel_does_not_move_or_end_a_left_window_capture() {
    let (mut state, registry, pin, frame) = fixture();
    arm(&mut state);
    let binding = state.window.selection.as_ref().unwrap().binding;
    let mut wheel = pin.down.pointer;
    wheel.kind = PointerKind::Wheel;
    wheel.button = None;
    wheel.delta = 30;
    wheel.window.0 = 110;
    registry.input(Monitor::Timeline, 7, 1, wheel, Some(frame.clone()));
    assert!(registry.events(&pin).is_empty());
    state.window_events(
        1,
        registry.events(&pin),
        registry.current(Monitor::Timeline, 1),
    );
    assert!(state.window.active.is_some());
    assert_eq!(state.history.revision(), 0);
    movement(&mut state, &registry, &pin, &frame, 92);
    wheel.kind = PointerKind::Up;
    wheel.button = Some(Button::Left);
    wheel.buttons = 0;
    registry.input(Monitor::Timeline, 7, 1, wheel, Some(frame));
    state.window_events(
        1,
        registry.events(&pin),
        registry.current(Monitor::Timeline, 1),
    );
    assert!(state.window.active.is_none());
    assert_eq!(state.history.revision(), 1);
    state.window_end("view_changed");
    assert_eq!(
        state.history.revision(),
        1,
        "navigation after native up does not create history"
    );
    assert_eq!(state.window.selection.as_ref().unwrap().binding, binding);
}

#[test]
fn native_move_before_up_is_retained_and_up_does_not_add_position() {
    let (mut state, registry, pin, frame) = fixture();
    arm(&mut state);
    let mut p = pin.down.pointer;
    p.kind = PointerKind::Move;
    p.button = None;
    p.window.0 = 92;
    registry.input(Monitor::Timeline, 7, 1, p, Some(frame.clone()));
    p.kind = PointerKind::Up;
    p.button = Some(Button::Left);
    p.buttons = 0;
    p.window.0 = 110;
    registry.input(Monitor::Timeline, 7, 1, p, Some(frame));
    state.window_events(
        1,
        registry.events(&pin),
        registry.current(Monitor::Timeline, 1),
    );
    assert!(state.window.active.is_none());
    assert_eq!(state.history.revision(), 1);
    let selection = state.window.selection.as_ref().unwrap();
    let Param::Keys(keys) = &selection
        .corrector(state.history.project())
        .unwrap()
        .windows[0]
        .transform
        .pan
    else {
        panic!()
    };
    assert_eq!(keys[0].value, 62.5);
    assert_eq!(keys[1].value, 70.0);
}

#[test]
fn echoed_up_cannot_overtake_a_move_copied_by_the_task_waiting_for_state() {
    let (mut state, registry, pin, frame) = fixture();
    arm(&mut state);
    let mut p = pin.down.pointer;
    p.kind = PointerKind::Move;
    p.button = None;
    p.window.0 = 92;
    registry.input(Monitor::Timeline, 7, 1, p, Some(frame.clone()));
    let delayed = registry.events(&pin);
    p.kind = PointerKind::Up;
    p.button = Some(Button::Left);
    p.buttons = 0;
    p.window.0 = 110;
    registry.input(Monitor::Timeline, 7, 1, p, Some(frame));
    // The RPC wins State while the task still holds its cloned move evidence.
    state.window_events(
        1,
        registry.events(&pin),
        registry.current(Monitor::Timeline, 1),
    );
    state.window_events(1, delayed, registry.current(Monitor::Timeline, 1));
    assert_eq!(
        state.history.revision(),
        1,
        "the held move must commit before up, whichever caller acquires State first"
    );
    assert!(state.window.active.is_none());
}

#[test]
fn external_intent_retiring_the_pin_rejects_a_move_already_copied_by_the_task() {
    let (mut state, registry, pin, frame) = fixture();
    arm(&mut state);
    let mut p = pin.down.pointer;
    p.kind = PointerKind::Move;
    p.window.0 = 92;
    registry.input(Monitor::Timeline, 7, 1, p, Some(frame));
    let delayed = registry.events(&pin);
    registry.stop_input(Monitor::Timeline);
    state.window_events(1, delayed, registry.current(Monitor::Timeline, 1));
    assert_eq!(
        state.history.revision(),
        0,
        "external intent ends at the last successful move"
    );
    assert!(state.window.active.is_none());
}

#[test]
fn native_pin_survives_pending_frame_eviction_and_rejects_replayed_down() {
    let (mut state, registry, pin, frame) = fixture();
    for _ in 0..20 {
        registry.register(Monitor::Timeline, 1, frame.clone());
    }
    assert!(registry.claim(7, pin.down.sequence).is_none());
    assert!(pin.live.load(Ordering::Acquire));
    state.window_end("end");
    assert!(!pin.live.load(Ordering::Acquire));
}

#[tokio::test]
async fn copying_the_same_presentation_does_not_self_wake_an_idle_capture() {
    let (_, registry, pin, _) = fixture();
    registry.observe(Monitor::Timeline, 1, pin.down.pointer.presentation);
    assert!(
        tokio::time::timeout(Duration::from_millis(5), pin.wake.notified())
            .await
            .is_err(),
        "a synchronous copy must not schedule its own next poll"
    );
}

#[test]
fn outline_evaluates_interpolation_while_move_edits_only_the_original_left_key() {
    let (state, _, _, frame) = fixture();
    let selection = state.window.selection.as_ref().unwrap();
    let original = &selection.corrector(&frame.project).unwrap().windows[0];
    let evaluated = original.transform.pan.value_at(10.0);
    let overlay = selection.overlay(&frame, None).unwrap();
    assert!((overlay.shapes[0].to_canvas.tx - evaluated * 1.28).abs() < 1e-10);
    let movement = Movement::new(selection, &frame, "w1", (64.0, 36.0)).unwrap();
    let moved = movement.at((80.0, 36.0)).unwrap();
    let Param::Keys(keys) = &moved[0].transform.pan else {
        panic!()
    };
    assert_eq!(keys[0].value, 62.5);
    assert_eq!(
        keys[0].interp,
        match &original.transform.pan {
            Param::Keys(keys) => keys[0].interp,
            Param::Fixed(_) => panic!(),
        }
    );
    assert_eq!(
        keys[1],
        match &original.transform.pan {
            Param::Keys(keys) => keys[1].clone(),
            Param::Fixed(_) => panic!(),
        }
    );
    assert_eq!(moved[1], movement.original[1]);
    assert!(
        moved[0].transform.pan.value_at(10.0) - evaluated < 12.5,
        "no compensation for interpolation"
    );
    assert_eq!(movement.clock, "source");
    for (at, expected) in [(-100, 0), (0, 0), (19, 0), (20, 20), (100, 20)] {
        assert_eq!(
            crate::window::LeftKey::of(&original.transform.pan, at).frame,
            Some(expected)
        );
    }
}

#[test]
fn oriented_nonsquare_picture_and_ninety_degree_placement_use_named_affine_order() {
    let (_, _, _, original) = fixture();
    let mut frame = (*original).clone();
    let selection = frame.selection.as_ref().unwrap().clone();
    let project = Arc::make_mut(&mut frame.project);
    let NodeKind::Corrector(c) = &mut project.timelines[0].tracks[0].clips[0].grade.nodes[0].kind
    else {
        panic!()
    };
    c.windows[0].transform.pan = Param::Fixed(50.0);
    c.windows[0].transform.tilt = Param::Fixed(50.0);
    c.windows[0].transform.rotate = Param::Fixed(90.0);
    frame.canvas = (1280, 720);
    frame.layers[0].shown_size = (360, 640);
    frame.layers[0].picture_to_canvas = [0.0, -360.0, 202.5, 0.0, 948.0, 258.75];
    let overlay = selection.overlay(&frame, None).unwrap();
    let a = overlay.shapes[0].to_canvas;
    for (actual, expected) in [a.xx, a.xy, a.yx, a.yy, a.tx, a.ty]
        .into_iter()
        .zip([-360.0, 0.0, 0.0, -360.0, 768.0, 360.0])
    {
        assert!((actual - expected).abs() < 1e-9, "{actual} != {expected}");
    }
    let movement = Movement::new(&selection, &frame, "w1", (768.0, 360.0)).unwrap();
    let moved = movement.at((768.0, 380.25)).unwrap();
    assert_eq!(moved[0].transform.pan, Param::Fixed(60.0));
    assert_eq!(moved[0].transform.tilt, Param::Fixed(50.0));
}

#[test]
fn scene_selection_mismatches_hidden_split_and_nonexact_sources_suppress_overlay() {
    let (_, _, _, original) = fixture();
    for case in 0..10 {
        let mut frame = (*original).clone();
        let mut selection = frame.selection.as_ref().unwrap().as_ref().clone();
        match case {
            0 => selection.visible = false,
            1 => selection.outline = false,
            2 => selection.tool = Tool::ImageWipe,
            3 => frame.revision += 1,
            4 => frame.comparison = json!({"mode":"split"}),
            5 => frame.layers.clear(),
            6 => frame.layers[0].editable = false,
            7 => frame.ready = false,
            8 => selection.clip = Some("c2".into()),
            9 => {
                frame.highlight = Some(crate::highlight::Highlight {
                    mode: crate::highlight::Mode::Bw,
                    grade: selection.target.clone(),
                    node: "other".into(),
                });
            }
            _ => unreachable!(),
        }
        assert!(selection.overlay(&frame, None).is_none(), "case {case}");
    }
}

#[test]
fn group_membership_is_required_and_timeline_grade_uses_canvas_clock() {
    let (_, _, _, original) = fixture();
    let mut frame = (*original).clone();
    let mut selection = frame.selection.as_ref().unwrap().as_ref().clone();
    let project = Arc::make_mut(&mut frame.project);
    let grade = project.timelines[0].tracks[0].clips[0].grade.clone();
    let mut group = columbina_model::grade::Group::new("g1", "group");
    group.pre = grade.clone();
    group.post = grade.clone();
    project.groups.push(group);
    project.timelines[0].grade = grade;
    for target in [
        Target::PreClip { group: "g1".into() },
        Target::PostClip { group: "g1".into() },
    ] {
        selection.target = target;
        assert!(selection.overlay(&frame, None).is_none());
        Arc::make_mut(&mut frame.project).timelines[0].tracks[0].clips[0].group = Some("g1".into());
        assert!(selection.overlay(&frame, None).is_some());
        Arc::make_mut(&mut frame.project).timelines[0].tracks[0].clips[0].group = None;
    }
    selection.target = Target::Timeline {
        timeline: "t1".into(),
    };
    selection.clip = None;
    let movement = Movement::new(&selection, &frame, "w1", (64.0, 36.0)).unwrap();
    assert_eq!(movement.clock, "timeline");
    assert_eq!(movement.frame, 10);
}

#[test]
fn a_captured_move_crosses_actual_wipe_coverage_and_old_instances_end_it() {
    let (mut state, registry, pin, frame) = fixture();
    arm(&mut state);
    let mut next = (*frame).clone();
    next.overlay = next.selection.as_ref().unwrap().overlay(
        &next,
        Some(columbina_compose::wipe::Wipe {
            position: 0.5,
            ..Default::default()
        }),
    );
    let next = Arc::new(next);
    let tag = registry
        .register(Monitor::Timeline, 1, next.clone())
        .unwrap();
    let shown = Presentation {
        tag: Some(tag),
        serial: 2,
        ..pin.down.pointer.presentation.unwrap()
    };
    registry.observe(Monitor::Timeline, 1, Some(shown));
    let mut p = pin.down.pointer;
    p.kind = PointerKind::Move;
    p.window = (110, 36);
    p.presentation = Some(shown);
    registry.input(Monitor::Timeline, 7, 1, p, Some(next));
    state.window_events(
        1,
        registry.events(&pin),
        registry.current(Monitor::Timeline, 1),
    );
    assert_eq!(state.history.revision(), 1);
    assert!(state.window.active.is_some());
    registry.activate(Monitor::Timeline, Some(2));
    state.window_events(1, vec![], registry.current(Monitor::Timeline, 1));
    assert!(state.window.active.is_none());
    assert!(!pin.live.load(Ordering::Acquire));
}

#[test]
fn clicking_a_different_circle_reports_its_local_id_without_rebinding() {
    let (mut state, registry, _, original) = fixture();
    state.window_end("previous");
    let mut frame = (*original).clone();
    let project = Arc::make_mut(&mut frame.project);
    let NodeKind::Corrector(c) = &mut project.timelines[0].tracks[0].clips[0].grade.nodes[0].kind
    else {
        panic!()
    };
    c.windows[1] = Window::new("w2", WindowShape::circular());
    c.windows[1].transform.pan = Param::Fixed(85.0);
    state.history = History::new(project.clone());
    frame.overlay = frame.selection.as_ref().unwrap().overlay(&frame, None);
    let frame = Arc::new(frame);
    let tag = registry
        .register(Monitor::Timeline, 1, frame.clone())
        .unwrap();
    let shown = Presentation {
        serial: 2,
        tag: Some(tag),
        map: columbina_view::Viewport::Fit.map((128, 72), (128, 72), 0),
    };
    registry.observe(Monitor::Timeline, 1, Some(shown));
    let pointer = columbina_view::Pointer {
        kind: PointerKind::Down,
        delta: 0,
        clicks: 1,
        middle_moved: false,
        button: Some(Button::Left),
        buttons: 1,
        inside: true,
        picture: Some((108.5, 36.5)),
        window: (108, 36),
        presentation: Some(shown),
        shift: false,
        ctrl: false,
        alt: false,
    };
    let event = registry.input(Monitor::Timeline, 7, 1, pointer, Some(frame));
    let pin = registry.claim(7, event).unwrap();
    let result = state.window_begin(7, 1, pin, registry).unwrap();
    assert_eq!(result["selected"], "w2");
    assert_eq!(result["binding"], 1);
    assert_eq!(
        state.window.selection.as_ref().unwrap().selected.as_deref(),
        Some("w2")
    );
}

#[test]
fn post_arm_moves_still_reject_a_changed_event_frame_or_source() {
    for frame_changed in [true, false] {
        let (mut state, registry, pin, frame) = fixture();
        let before = state.history.project().clone();
        arm(&mut state);
        let mut changed = (*frame).clone();
        if frame_changed {
            changed.timeline_frame += 1;
        } else {
            changed.layers[0].origin.key_frame += 1;
        }
        // The actual current presentation is still the clicked frame. Only this event
        // is stale, and its sequence is after the successful arming boundary.
        movement(&mut state, &registry, &pin, &Arc::new(changed), 84);
        assert!(state.window.active.is_none());
        assert_eq!(state.history.project(), &before);
        assert_eq!(state.history.revision(), 0);
        assert!(!state.history.can_undo());
    }
}

#[test]
fn map_frame_placement_unknown_tag_gone_and_disconnect_finish_at_the_last_success() {
    for case in 0..6 {
        let (mut state, registry, pin, frame) = fixture();
        let before = state.history.project().clone();
        arm(&mut state);
        movement(&mut state, &registry, &pin, &frame, 84);
        let last = state.history.project().clone();
        let mut current = registry.current(Monitor::Timeline, 1).unwrap();
        match case {
            0 => current.0.map.as_mut().unwrap().dest[0] += 1.0,
            1 => Arc::make_mut(&mut current.1).timeline_frame += 1,
            2 => Arc::make_mut(&mut current.1).layers[0].picture_to_canvas[4] += 1.0,
            3 => {
                let shown = Presentation {
                    serial: 2,
                    tag: Some(9999),
                    ..current.0
                };
                registry.observe(Monitor::Timeline, 1, Some(shown));
            }
            4 => registry.gone(Monitor::Timeline, 1),
            5 => state.window_unbind("disconnect"),
            _ => unreachable!(),
        }
        let current = if case < 3 {
            Some(current)
        } else {
            registry.current(Monitor::Timeline, 1)
        };
        state.window_events(1, vec![], current);
        assert!(state.window.active.is_none(), "case {case}");
        assert!(!pin.live.load(Ordering::Acquire));
        assert_eq!(state.history.project(), &last);
        state.history.undo("desktop").unwrap().unwrap();
        assert_eq!(state.history.project(), &before);
    }
}

#[tokio::test]
async fn a_valid_external_seek_before_the_orders_turn_cannot_be_rewound_by_late_freeze() {
    let (mut state, registry, _, _) = fixture();
    let mut project = state.history.project().clone();
    project.timelines[0].tracks[0].clips[0].source = columbina_model::Source::Color {
        color: columbina_model::Rgba([80, 40, 20, 255]),
    };
    state.history = History::new(project);
    let graphics = crate::gpu::Graphics::new();
    let viewer = crate::view::Viewer::new(false, graphics.clone());
    let transport = Transport::new(
        false,
        crate::transport::Setup {
            sound: crate::audio::Kind::Null,
            fail_card_at: None,
            test_hooks: true,
        },
        &viewer,
        graphics.clone(),
        crate::grading::LutPlaces::with_library(std::path::Path::new("."), vec![]),
        Arc::new(|_| {}),
        Arc::new(|_, _| {}),
        Arc::new(|_, _| {}),
        &crate::scopes::Scopes::new(true, graphics),
    );
    let load =
        serde_json::value::to_raw_value(&json!({"monitor":"timeline","timeline":"t1"})).unwrap();
    transport
        .begin_load(state.history.project(), 0, Some(&load))
        .unwrap()
        .done()
        .await
        .unwrap();
    let state = Arc::new(Mutex::new(state));
    let turn = transport.window_turn().await;
    assert!(state.lock().await.window_freeze_request(1).is_ok());
    let (started, queued) = tokio::sync::oneshot::channel();
    let waiting = tokio::spawn({
        let state = state.clone();
        let transport = transport.clone();
        async move {
            let _ = started.send(());
            let _turn = transport.window_turn().await;
            let state = state.lock().await;
            // Exercise the actual token gate used by Task::arm, without a missing-view check
            // masking an accidental late enqueue. The worker and orders queue are real.
            let (timeline, revision, frame) = state.window_freeze_request(1)?;
            transport
                .freeze_at(timeline, revision, frame, 1)
                .map(|_| ())
        }
    });
    queued.await.unwrap();
    let seek = serde_json::value::to_raw_value(&json!({"monitor":"timeline","frame":30})).unwrap();
    let (_, order) = transport.prepare("transport.seek", Some(&seek)).unwrap();
    // The RPC's validated intent invalidates arming before waiting for orders.
    registry.stop_input(Monitor::Timeline);
    state.lock().await.window_end("transport");
    drop(turn);
    order.send().await;
    assert_eq!(
        waiting.await.unwrap().unwrap_err().data.unwrap()["reason"],
        "capture"
    );
    let query = serde_json::value::to_raw_value(&json!({"monitor":"timeline"})).unwrap();
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        let (status, _) = transport.prepare("transport.state", Some(&query)).unwrap();
        if status["frame"] == 30 {
            break;
        }
        assert!(Instant::now() < until, "seek not processed: {status}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        transport
            .prepare("transport.state", Some(&query))
            .unwrap()
            .0["frame"],
        30
    );
    assert_eq!(state.lock().await.history.revision(), 0);
    transport.stop();
}

// ---- Curve drawing and single-point editing (T23 step 4, RM20 pp. 3193-3194) ----

type CurveAnchor = ([f64; 2], [f64; 2], [f64; 2]);

fn curve_window(anchors: &[CurveAnchor], closed: bool) -> Window {
    Window::new(
        "w1",
        WindowShape::Curve {
            points: anchors
                .iter()
                .map(|&(p, i, o)| columbina_model::grade::WindowBezierPoint {
                    position: WindowPoint::new(p[0], p[1]),
                    into: WindowPoint::new(i[0], i[1]),
                    out: WindowPoint::new(o[0], o[1]),
                })
                .collect(),
            closed,
            inside: Param::Fixed(0.0),
            outside: Param::Fixed(0.0),
        },
    )
}

/// The fixture's rectangle as Curve anchors (device 46..82 x 18..54); anchor 2 has handles
/// along its two edges.
fn curve_square() -> Vec<CurveAnchor> {
    vec![
        ([-0.25, -0.25], [0.0; 2], [0.0; 2]),
        ([0.25, -0.25], [0.0; 2], [0.0; 2]),
        ([0.25, 0.25], [0.0, -0.125], [-0.125, 0.0]),
        ([-0.25, 0.25], [0.0; 2], [0.0; 2]),
    ]
}

fn curve_points(window: &Window) -> (Vec<columbina_model::grade::WindowBezierPoint>, bool) {
    match &window.shape {
        WindowShape::Curve { points, closed, .. } => (points.clone(), *closed),
        _ => panic!("not a Curve"),
    }
}

fn fixed(param: &Param) -> f64 {
    match param {
        Param::Fixed(v) => *v,
        Param::Keys(_) => panic!("keyed {param:?}"),
    }
}

/// A down with Alt and/or the system's double click on a control of the selected `window`.
#[allow(clippy::type_complexity, reason = "one fixture tuple")]
fn try_curve(
    window: Window,
    at: (i32, i32),
    alt: bool,
    clicks: u8,
) -> (
    State,
    Registry,
    Arc<Pin>,
    Arc<FrameSnapshot>,
    Result<Value, Fault>,
) {
    try_curve_selecting(window, at, alt, clicks, Some("w1"))
}

/// As `try_curve`, with the binding selecting `selected` (none after Esc left a draft).
#[allow(clippy::type_complexity, reason = "one fixture tuple")]
fn try_curve_selecting(
    window: Window,
    at: (i32, i32),
    alt: bool,
    clicks: u8,
    selected: Option<&str>,
) -> (
    State,
    Registry,
    Arc<Pin>,
    Arc<FrameSnapshot>,
    Result<Value, Fault>,
) {
    try_curve_shown(window, at, alt, clicks, selected, None)
}

/// As `try_curve_selecting`, optionally with the clip's picture `size` (also the canvas) shown
/// through `map` instead of the fixture's 128x72 picture at one device pixel per canvas pixel.
#[allow(clippy::type_complexity, reason = "one fixture tuple")]
fn try_curve_shown(
    window: Window,
    at: (i32, i32),
    alt: bool,
    clicks: u8,
    selected: Option<&str>,
    shown: Option<((u32, u32), columbina_view::ViewportMap)>,
) -> (
    State,
    Registry,
    Arc<Pin>,
    Arc<FrameSnapshot>,
    Result<Value, Fault>,
) {
    let (mut state, registry, old_pin, old_frame) = fixture();
    state.window_end("fixture");
    let mut frame = (*old_frame).clone();
    let mut project = (*frame.project).clone();
    let NodeKind::Corrector(c) = &mut project.timelines[0].tracks[0].clips[0].grade.nodes[0].kind
    else {
        panic!()
    };
    c.windows[0] = window;
    let mut pointer = old_pin.down.pointer;
    if let Some((size, map)) = shown {
        project.timelines[0].width = size.0;
        project.timelines[0].height = size.1;
        frame.canvas = size;
        frame.layers[0].shown_size = size;
        frame.layers[0].picture_to_canvas =
            [f64::from(size.0), 0.0, 0.0, f64::from(size.1), 0.0, 0.0];
        pointer.presentation.as_mut().unwrap().map = Some(map);
    }
    state.history = History::new(project.clone());
    frame.project = Arc::new(project);
    Arc::make_mut(state.window.selection.as_mut().unwrap()).selected = selected.map(Into::into);
    frame.overlay = state
        .window
        .selection
        .as_ref()
        .unwrap()
        .overlay(&frame, None);
    let frame = Arc::new(frame);
    let tag = registry
        .register(Monitor::Timeline, 1, frame.clone())
        .unwrap();
    pointer.window = at;
    pointer.alt = alt;
    pointer.clicks = clicks;
    pointer.presentation.as_mut().unwrap().tag = Some(tag);
    pointer.presentation.as_mut().unwrap().serial += 1;
    registry.observe(Monitor::Timeline, 1, pointer.presentation);
    let event = registry.input(Monitor::Timeline, 7, 1, pointer, Some(frame.clone()));
    let pin = registry.claim(7, event).unwrap();
    let begun = state.window_begin(7, 1, pin.clone(), registry.clone());
    (state, registry, pin, frame, begun)
}

fn control_of(state: &State) -> Control {
    state.window.active.as_ref().unwrap().movement.control
}

#[test]
fn drawing_adds_a_corner_on_a_click_and_a_bezier_point_on_a_drag_each_one_undo_step() {
    let (mut state, registry, pin, frame, begun) =
        try_curve(curve_window(&[], false), (40, 36), false, 1);
    begun.unwrap();
    assert_eq!(control_of(&state), Control::Append);
    assert_eq!(
        state.window.active.as_ref().unwrap().movement.keys(),
        json!({"points":[{"point":0,"out_x":null,"out_y":null,"in_x":null,"in_y":null}],
            "inserted":0,"clock":"source","frame":10})
    );
    arm_capture(&mut state);
    // A wobble shorter than the pull threshold keeps a corner.
    drag(&mut state, &registry, &pin, &frame, (41, 36));
    let (points, closed) = curve_points(&first_window(&state));
    assert!(!closed);
    assert_eq!(points.len(), 1);
    assert!((fixed(&points[0].position.x) - (40.5 - 64.0) / 72.0).abs() < 1e-12);
    assert!((fixed(&points[0].position.y) - 0.5 / 72.0).abs() < 1e-12);
    assert_eq!(points[0].out, WindowPoint::new(0.0, 0.0));
    state.window_end("up");
    assert_eq!(state.history.revision(), 1);
    // A second point dragged out: the out handle follows, the in handle mirrors it.
    let drawn = first_window(&state);
    let (mut state, registry, pin, frame, begun) = try_curve(drawn, (88, 36), false, 1);
    begun.unwrap();
    arm_capture(&mut state);
    drag(&mut state, &registry, &pin, &frame, (100, 45));
    let (points, _) = curve_points(&first_window(&state));
    assert_eq!(points.len(), 2);
    assert!((fixed(&points[1].out.x) - 12.0 / 72.0).abs() < 1e-12);
    assert!((fixed(&points[1].out.y) - 9.0 / 72.0).abs() < 1e-12);
    assert!((fixed(&points[1].into.x) + 12.0 / 72.0).abs() < 1e-12);
    assert!((fixed(&points[1].into.y) + 9.0 / 72.0).abs() < 1e-12);
    state.window_end("up");
    state.history.undo("desktop").unwrap().unwrap();
    assert_eq!(curve_points(&first_window(&state)).0.len(), 1, "one step");
}

#[test]
fn the_pull_threshold_is_four_device_pixels_in_a_fit_4k_view_and_at_800_percent() {
    use columbina_view::Viewport;
    // A 3840x2160 picture fitted to 1920x1080: half a device pixel per canvas pixel.
    let fit = Viewport::Fit.map((1920, 1080), (3840, 2160), 0).unwrap();
    // The fixture's 128x72 picture at 800%: eight device pixels per canvas pixel.
    let zoomed = Viewport::Scale {
        scale: 8.0,
        center: [0.5, 0.5],
    }
    .map((128, 72), (128, 72), 0)
    .unwrap();
    for (size, map, at) in [
        ((3840, 2160), fit, (900, 540)),
        ((128, 72), zoomed, (40, 36)),
    ] {
        for (travel, pulled) in [(3, false), (4, true)] {
            let (mut state, registry, pin, frame, begun) = try_curve_shown(
                curve_window(&[], false),
                at,
                false,
                1,
                Some("w1"),
                Some((size, map)),
            );
            begun.unwrap();
            assert_eq!(control_of(&state), Control::Append);
            arm_capture(&mut state);
            drag(&mut state, &registry, &pin, &frame, (at.0 + travel, at.1));
            let (points, _) = curve_points(&first_window(&state));
            assert_eq!(points.len(), 1);
            assert_eq!(
                fixed(&points[0].out.x) > 0.0,
                pulled,
                "{travel} device px in {size:?}: {:?}",
                points[0]
            );
            assert_eq!(fixed(&points[0].out.y), 0.0);
        }
    }
}

#[test]
fn a_down_just_outside_a_closed_curve_moves_it_and_one_beside_a_draft_draws() {
    // The square's top edge is device y = 18; pixel centre 64.5, 14.5 is 3.5 pixels above it.
    for selected in [Some("w1"), None] {
        let (mut state, registry, pin, frame, begun) = try_curve_selecting(
            curve_window(&curve_square(), true),
            (64, 14),
            false,
            1,
            selected,
        );
        begun.unwrap();
        assert_eq!(control_of(&state), Control::Move);
        assert_eq!(
            state.window.selection.as_ref().unwrap().selected.as_deref(),
            Some("w1")
        );
        arm_capture(&mut state);
        drag(&mut state, &registry, &pin, &frame, (70, 20));
        let moved = first_window(&state);
        let before = curve_window(&curve_square(), true);
        let panned = moved.transform.pan.value_at(10.0) - before.transform.pan.value_at(10.0);
        assert!((panned - 6.0 / 128.0 * 100.0).abs() < 1e-9, "{panned}");
        assert_eq!(
            curve_points(&moved).0,
            curve_points(&before).0,
            "the whole window moves, not a point"
        );
    }
    // Two pixels further out nothing is hit; beside a selected draft the down draws.
    let (_, _, _, _, begun) = try_curve(curve_window(&curve_square(), true), (64, 12), false, 1);
    assert_eq!(begun.unwrap_err().data, Some(json!({"reason":"hit"})));
    let (state, _, _, _, begun) =
        try_curve(curve_window(&curve_square(), false), (64, 14), false, 1);
    begun.unwrap();
    assert_eq!(control_of(&state), Control::Append);
}

#[test]
fn esc_during_a_drawing_drag_removes_only_that_point_and_keeps_the_draft() {
    let (mut state, registry, pin, frame, begun) = try_curve(
        curve_window(&curve_square()[..2], false),
        (64, 54),
        false,
        1,
    );
    begun.unwrap();
    let before = state.history.project().clone();
    arm_capture(&mut state);
    drag(&mut state, &registry, &pin, &frame, (70, 60));
    assert_eq!(curve_points(&first_window(&state)).0.len(), 3);
    let capture = state.window.active.take().unwrap();
    state.window_cancel(7, capture);
    assert_eq!(state.history.project(), &before);
    let (points, closed) = curve_points(&first_window(&state));
    assert_eq!((points.len(), closed), (2, false), "the draft stays");
    assert!(!state.history.can_undo());
}

#[test]
fn clicking_the_first_point_closes_and_dragging_it_closes_with_bezier_handles() {
    // The first point was drawn as a Bezier point: a click still closes with a corner there.
    let mut anchors = curve_square();
    anchors[0].1 = [0.0, 0.1];
    anchors[0].2 = [0.1, 0.0];
    for dragged in [false, true] {
        let (mut state, registry, pin, frame, begun) =
            try_curve(curve_window(&anchors, false), (46, 18), false, 1);
        begun.unwrap();
        assert_eq!(control_of(&state), Control::Close);
        arm_capture(&mut state);
        if dragged {
            drag(&mut state, &registry, &pin, &frame, (46, 30));
        }
        let (points, closed) = curve_points(&first_window(&state));
        assert!(closed);
        assert_eq!(points.len(), 4, "closing adds no point");
        let out = [fixed(&points[0].out.x), fixed(&points[0].out.y)];
        let into = [fixed(&points[0].into.x), fixed(&points[0].into.y)];
        if dragged {
            assert!((out[1] - 12.0 / 72.0).abs() < 1e-12 && out[0].abs() < 1e-12);
            assert!((into[1] + 12.0 / 72.0).abs() < 1e-12);
        } else {
            assert_eq!((out, into), ([0.0; 2], [0.0; 2]), "a corner closing");
        }
        state.window_end("up");
        state.history.undo("desktop").unwrap().unwrap();
        assert!(!curve_points(&first_window(&state)).1, "one step");
    }
    // A released click before arming still closes, with its corner, as one step.
    let (mut state, registry, pin, frame, begun) =
        try_curve(curve_window(&anchors, false), (46, 18), false, 1);
    begun.unwrap();
    let mut pointer = pin.down.pointer;
    pointer.kind = PointerKind::Up;
    pointer.buttons = 0;
    registry.input(Monitor::Timeline, 7, 1, pointer, Some(frame.clone()));
    let capture = state.window.active.take().unwrap();
    state.window_finish_unarmed(capture, "native_end");
    let (points, closed) = curve_points(&first_window(&state));
    assert!(closed);
    assert_eq!(points[0].out, WindowPoint::new(0.0, 0.0));
    assert_eq!(state.history.revision(), 1);
}

#[test]
fn a_linked_handle_drag_turns_both_keeping_angle_and_ratio_and_alt_moves_one() {
    // Anchor 2 (device 82, 54): in handle up 9 px, out handle left 9 px; make them unequal.
    let mut anchors = curve_square();
    anchors[2].2 = [-0.25, 0.0];
    for alt in [false, true] {
        let (mut state, registry, pin, frame, begun) =
            try_curve(curve_window(&anchors, true), (82, 45), alt, 1);
        begun.unwrap();
        assert_eq!(control_of(&state), Control::HandleIn(2));
        arm_capture(&mut state);
        // Turn the in handle a quarter turn clockwise on screen and double it: up 9 -> right 18.
        drag(&mut state, &registry, &pin, &frame, (100, 54));
        let (points, _) = curve_points(&first_window(&state));
        let into = [fixed(&points[2].into.x), fixed(&points[2].into.y)];
        let out = [fixed(&points[2].out.x), fixed(&points[2].out.y)];
        assert!(
            (into[0] - 0.25).abs() < 1e-12 && into[1].abs() < 1e-12,
            "{into:?}"
        );
        if alt {
            assert_eq!(out, [-0.25, 0.0], "Alt leaves the opposite handle");
        } else {
            // The opposite handle turned the same quarter and doubled: left .25 -> up .5.
            assert!(
                out[0].abs() < 1e-12 && (out[1] + 0.5).abs() < 1e-12,
                "{out:?}"
            );
        }
        assert_eq!(points[2].position, WindowPoint::new(0.25, 0.25));
    }
}

#[test]
fn a_zero_opposite_handle_stays_zero_and_alt_drag_on_a_corner_pulls_handles() {
    let mut anchors = curve_square();
    anchors[2].1 = [0.0; 2];
    let (mut state, registry, pin, frame, begun) =
        try_curve(curve_window(&anchors, true), (73, 54), false, 1);
    begun.unwrap();
    assert_eq!(control_of(&state), Control::HandleOut(2));
    arm_capture(&mut state);
    drag(&mut state, &registry, &pin, &frame, (73, 63));
    let (points, _) = curve_points(&first_window(&state));
    assert_eq!(points[2].into, WindowPoint::new(0.0, 0.0), "a corner side");
    // Alt on a corner anchor pulls a symmetric pair; a plain drag would move the anchor.
    let (mut state, registry, pin, frame, begun) =
        try_curve(curve_window(&curve_square(), true), (82, 18), true, 1);
    begun.unwrap();
    arm_capture(&mut state);
    drag(&mut state, &registry, &pin, &frame, (91, 18));
    let (points, _) = curve_points(&first_window(&state));
    assert_eq!(points[1].position, WindowPoint::new(0.25, -0.25));
    assert!((fixed(&points[1].out.x) - 0.125).abs() < 1e-12);
    assert!((fixed(&points[1].into.x) + 0.125).abs() < 1e-12);
}

#[test]
fn alt_double_click_makes_a_bezier_point_a_corner_in_one_step() {
    let (mut state, _, _, _, begun) =
        try_curve(curve_window(&curve_square(), true), (82, 54), true, 2);
    begun.unwrap();
    assert_eq!(control_of(&state), Control::Point(2));
    arm_capture(&mut state);
    let (points, _) = curve_points(&first_window(&state));
    assert_eq!(points[2].into, WindowPoint::new(0.0, 0.0));
    assert_eq!(points[2].out, WindowPoint::new(0.0, 0.0));
    assert_eq!(points[2].position, WindowPoint::new(0.25, 0.25));
    state.window_end("up");
    assert_eq!(state.history.revision(), 1);
}

#[test]
fn a_curve_edge_click_inserts_on_the_original_cubic_and_its_drag_moves_the_new_point() {
    let (mut state, registry, pin, frame, begun) =
        try_curve(curve_window(&curve_square(), true), (64, 18), false, 1);
    begun.unwrap();
    assert_eq!(control_of(&state), Control::Insert(0));
    arm_capture(&mut state);
    let (points, _) = curve_points(&first_window(&state));
    assert_eq!(points.len(), 5);
    // On the straight top edge, at the clicked x (pixel centre 64.5).
    assert!((fixed(&points[1].position.y) + 0.25).abs() < 1e-9);
    assert!((fixed(&points[1].position.x) - 0.5 / 72.0).abs() < 1e-6);
    drag(&mut state, &registry, &pin, &frame, (64, 27));
    let (points, _) = curve_points(&first_window(&state));
    assert!((fixed(&points[1].position.y) + 0.125).abs() < 1e-9);
    state.window_end("up");
    state.history.undo("desktop").unwrap().unwrap();
    assert_eq!(curve_points(&first_window(&state)).0.len(), 4);
}

#[test]
fn curve_softness_handles_drag_inside_and_outside_softness() {
    let mut window = curve_window(&curve_square(), true);
    let WindowShape::Curve {
        inside, outside, ..
    } = &mut window.shape
    else {
        panic!()
    };
    *inside = Param::Fixed(20.0);
    *outside = Param::Fixed(20.0);
    // .1 image heights each: 7.2 device pixels. Edge 0 at t = 1/3 is x = 46 + 36 * .259.
    let (mut state, registry, pin, frame, begun) = try_curve(window.clone(), (55, 25), false, 1);
    begun.unwrap();
    assert_eq!(control_of(&state), Control::Softness(-1));
    arm_capture(&mut state);
    drag(&mut state, &registry, &pin, &frame, (55, 34));
    let WindowShape::Curve { inside, .. } = &first_window(&state).shape else {
        panic!()
    };
    assert!((fixed(inside) - 45.0).abs() < 1e-9, "{inside:?}");
    let (state, _, _, _, begun) = try_curve(window, (72, 10), false, 1);
    begun.unwrap();
    assert_eq!(control_of(&state), Control::Softness(1));
}

#[test]
fn a_middle_tap_deletes_a_curve_point_but_keeps_two_closed_and_empties_a_draft() {
    let (mut state, registry, pin, frame) =
        control_window_fixture(curve_window(&curve_square()[..3], true), (64, 36));
    state.window_end("fixture");
    let tap = middle_tap(&registry, &pin, &frame, (82, 18), false);
    assert_eq!(state.window_delete(7, 1, &tap).unwrap()["point"], 1);
    assert_eq!(curve_points(&first_window(&state)).0.len(), 2);
    let (mut state, registry, pin, frame) =
        control_window_fixture(curve_window(&curve_square()[..2], true), (64, 18));
    state.window_end("fixture");
    let tap = middle_tap(&registry, &pin, &frame, (82, 18), false);
    assert_eq!(
        state.window_delete(7, 1, &tap).unwrap_err().data,
        Some(json!({"reason":"delete","topology":{"why":"minimum","least":2,"shape":"curve"}}))
    );
    // A draft's first point (its Close control) is deletable.
    let (mut state, registry, pin, frame, begun) = try_curve(
        curve_window(&curve_square()[..2], false),
        (64, 18),
        false,
        1,
    );
    begun.unwrap();
    state.window_end("fixture");
    let tap = middle_tap(&registry, &pin, &frame, (46, 18), false);
    assert_eq!(state.window_delete(7, 1, &tap).unwrap()["point"], 0);
    let (points, closed) = curve_points(&first_window(&state));
    assert_eq!((points.len(), closed), (1, false));
}

#[test]
fn an_unselected_draft_is_selected_by_its_outline_and_nothing_else_draws_while_none_is() {
    let draft = curve_window(&curve_square()[..2], false);
    let (state, _, _, _, begun) = try_curve_selecting(draft.clone(), (64, 18), false, 1, None);
    begun.unwrap();
    assert_eq!(control_of(&state), Control::Move);
    assert_eq!(
        state.window.selection.as_ref().unwrap().selected.as_deref(),
        Some("w1"),
        "selected again, so its next click draws"
    );
    let (_, _, _, _, begun) = try_curve_selecting(draft, (64, 50), false, 1, None);
    assert_eq!(begun.unwrap_err().data, Some(json!({"reason":"hit"})));
}

// ---- Curve multi-point selection (T23 step 5, RM20 p. 3195) ----

/// A down at `at` (Shift held or not) on the selected Curve `window` whose anchors `chosen` are
/// already selected; the down's frame shows that selection.
#[allow(clippy::type_complexity, reason = "one fixture tuple")]
fn try_chosen(
    window: Window,
    at: (i32, i32),
    shift: bool,
    chosen: &[u32],
) -> (
    State,
    Registry,
    Arc<Pin>,
    Arc<FrameSnapshot>,
    Result<Value, Fault>,
) {
    try_chosen_wiped(window, at, shift, chosen, None, None)
}

/// As `try_chosen`, the frame compared through `wipe`, and shown through `map` instead of one
/// device pixel per canvas pixel.
#[allow(clippy::type_complexity, reason = "one fixture tuple")]
fn try_chosen_wiped(
    window: Window,
    at: (i32, i32),
    shift: bool,
    chosen: &[u32],
    wipe: Option<columbina_compose::wipe::Wipe>,
    map: Option<columbina_view::ViewportMap>,
) -> (
    State,
    Registry,
    Arc<Pin>,
    Arc<FrameSnapshot>,
    Result<Value, Fault>,
) {
    let (mut state, registry, old_pin, old_frame) = fixture();
    state.window_end("fixture");
    let mut frame = (*old_frame).clone();
    let mut project = (*frame.project).clone();
    let NodeKind::Corrector(c) = &mut project.timelines[0].tracks[0].clips[0].grade.nodes[0].kind
    else {
        panic!()
    };
    let count = match &window.shape {
        WindowShape::Curve { points, .. } => points.len(),
        _ => 0,
    };
    c.windows[0] = window;
    state.history = History::new(project.clone());
    frame.project = Arc::new(project);
    {
        let selection = Arc::make_mut(state.window.selection.as_mut().unwrap());
        selection.selected = Some("w1".into());
        selection.chosen = (!chosen.is_empty()).then(|| Chosen {
            window: "w1".into(),
            count,
            points: chosen.to_vec(),
        });
    }
    let selection = state.window.selection.clone().unwrap();
    frame.overlay = selection.overlay(&frame, wipe);
    frame.selection = Some(selection);
    let frame = Arc::new(frame);
    let tag = registry
        .register(Monitor::Timeline, 1, frame.clone())
        .unwrap();
    let mut pointer = old_pin.down.pointer;
    if map.is_some() {
        pointer.presentation.as_mut().unwrap().map = map;
    }
    pointer.window = at;
    pointer.shift = shift;
    pointer.presentation.as_mut().unwrap().tag = Some(tag);
    pointer.presentation.as_mut().unwrap().serial += 1;
    registry.observe(Monitor::Timeline, 1, pointer.presentation);
    let event = registry.input(Monitor::Timeline, 7, 1, pointer, Some(frame.clone()));
    let pin = registry.claim(7, event).unwrap();
    let begun = state.window_begin(7, 1, pin.clone(), registry.clone());
    (state, registry, pin, frame, begun)
}

/// The native left up at `to`, consumed as the task consumes it.
fn release(
    state: &mut State,
    registry: &Registry,
    pin: &Arc<Pin>,
    frame: &Arc<FrameSnapshot>,
    to: (i32, i32),
) {
    let mut pointer = pin.down.pointer;
    pointer.kind = PointerKind::Up;
    pointer.buttons = 0;
    pointer.window = to;
    registry.input(Monitor::Timeline, 7, 1, pointer, Some(frame.clone()));
    let id = state.window.active.as_ref().unwrap().id;
    state.window_events(
        id,
        registry.events(pin),
        registry.current(Monitor::Timeline, 1),
    );
}

fn chosen_points(state: &State) -> Vec<u32> {
    state.window.selection.as_ref().unwrap().points().to_vec()
}

/// The fixture square (device 46..82 x 18..54); anchor 1 also has handles, and its x is keyed
/// 0.2 at frame 0 to 0.3 at frame 20 (shown .25 at the fixture's frame 10).
fn chosen_square() -> Window {
    let mut anchors = curve_square();
    anchors[1].2 = [0.1, 0.05];
    anchors[1].1 = [-0.05, 0.0];
    let mut window = curve_window(&anchors, true);
    let WindowShape::Curve { points, .. } = &mut window.shape else {
        panic!()
    };
    points[1].position.x = Param::Keys(vec![
        Key {
            frame: 0,
            value: 0.2,
            interp: Interp::Linear,
        },
        Key {
            frame: 20,
            value: 0.3,
            interp: Interp::Linear,
        },
    ]);
    window
}

fn left_key(param: &Param) -> f64 {
    match param {
        Param::Fixed(v) => *v,
        Param::Keys(keys) => keys[0].value,
    }
}

#[test]
fn a_shift_drag_selects_the_anchors_its_band_encloses_without_history() {
    // A Shift down anywhere while the Curve is selected is a band, even on its edge.
    let (state, _, _, _, begun) = try_chosen(chosen_square(), (64, 18), true, &[]);
    begun.unwrap();
    assert_eq!(control_of(&state), Control::Band);
    let (mut state, registry, pin, frame, begun) = try_chosen(chosen_square(), (40, 10), true, &[]);
    begun.unwrap();
    arm_capture(&mut state);
    drag(&mut state, &registry, &pin, &frame, (88, 24));
    // While it is dragged the band is shown, from the down to the pointer, in canvas pixels.
    assert_eq!(
        state.window.selection.as_ref().unwrap().band,
        Some([40.5, 10.5, 88.5, 24.5])
    );
    assert_eq!(
        chosen_points(&state),
        Vec::<u32>::new(),
        "selected only on release"
    );
    release(&mut state, &registry, &pin, &frame, (88, 24));
    assert!(state.window.active.is_none());
    assert_eq!(chosen_points(&state), [0, 1]);
    assert_eq!(state.window.selection.as_ref().unwrap().band, None);
    assert_eq!(state.history.revision(), 0, "pure selection");
    assert!(!state.history.can_undo());
    // Esc during a band keeps the previous selection.
    let (mut state, registry, pin, frame, begun) =
        try_chosen(chosen_square(), (88, 60), true, &[0, 1]);
    begun.unwrap();
    arm_capture(&mut state);
    drag(&mut state, &registry, &pin, &frame, (40, 40));
    let capture = state.window.active.take().unwrap();
    state.window_cancel(7, capture);
    assert_eq!(chosen_points(&state), [0, 1]);
    assert_eq!(state.window.selection.as_ref().unwrap().band, None);
    // Released, a band upwards and leftwards replaces the selection.
    let (mut state, registry, pin, frame, begun) =
        try_chosen(chosen_square(), (88, 60), true, &[0, 1]);
    begun.unwrap();
    arm_capture(&mut state);
    release(&mut state, &registry, &pin, &frame, (40, 40));
    assert_eq!(chosen_points(&state), [2, 3]);
    // A Shift click released before arming selects the anchor under it (its radius counts).
    let (mut state, registry, pin, frame, begun) =
        try_chosen(chosen_square(), (48, 56), true, &[0, 1]);
    begun.unwrap();
    let mut pointer = pin.down.pointer;
    pointer.kind = PointerKind::Up;
    pointer.buttons = 0;
    registry.input(Monitor::Timeline, 7, 1, pointer, Some(frame.clone()));
    let capture = state.window.active.take().unwrap();
    state.window_finish_unarmed(capture, "up");
    assert_eq!(chosen_points(&state), [3]);
    // A band enclosing no anchor clears the selection.
    let (mut state, registry, pin, frame, begun) =
        try_chosen(chosen_square(), (60, 30), true, &[0, 1]);
    begun.unwrap();
    arm_capture(&mut state);
    release(&mut state, &registry, &pin, &frame, (70, 40));
    assert_eq!(chosen_points(&state), Vec::<u32>::new());
    assert!(state.window.selection.as_ref().unwrap().chosen.is_none());
    // Not on a pure reference part of a wipe, where no down is taken.
    let wipe = columbina_compose::wipe::Wipe {
        position: 0.5,
        ..Default::default()
    };
    let (_, _, _, _, begun) =
        try_chosen_wiped(chosen_square(), (110, 36), true, &[0, 1], Some(wipe), None);
    assert_eq!(begun.unwrap_err().data, Some(json!({"reason":"hit"})));
    let (state, _, _, _, begun) =
        try_chosen_wiped(chosen_square(), (20, 36), true, &[0, 1], Some(wipe), None);
    begun.unwrap();
    assert_eq!(control_of(&state), Control::Band);
}

#[test]
fn a_shift_click_takes_an_anchor_within_four_device_pixels_at_any_zoom() {
    use columbina_view::Viewport;
    // 800% about anchor 2 (canvas 82, 54): eight device pixels per canvas pixel.
    let map = Viewport::Scale {
        scale: 8.0,
        center: [82.0 / 128.0, 54.0 / 72.0],
    }
    .map((128, 72), (128, 72), 0)
    .unwrap();
    let anchor = map.to_device((82.0, 54.0));
    #[allow(
        clippy::cast_possible_truncation,
        reason = "device pixels near the surface"
    )]
    let pixel = |dx: f64| ((anchor.0 + dx).floor() as i32, anchor.1.floor() as i32);
    for (dx, taken) in [(3.0, true), (5.0, false)] {
        let (mut state, registry, pin, frame, begun) =
            try_chosen_wiped(chosen_square(), pixel(dx), true, &[], None, Some(map));
        begun.unwrap();
        arm_capture(&mut state);
        release(&mut state, &registry, &pin, &frame, pixel(dx));
        assert_eq!(
            chosen_points(&state),
            if taken { vec![2] } else { vec![] },
            "{dx} device px"
        );
    }
}

#[test]
fn the_box_moves_scales_stretches_and_rotates_the_selected_anchors_from_their_left_keys() {
    // Anchors 0 and 1 (canvas 46, 18 and 82, 18): the box centre is canvas (64, 18).
    let near = |a: f64, b: f64| (a - b).abs() < 1e-9;
    // Inside the box (on edge 0, which it covers) moves them; anchor 1's x writes its left key.
    let (mut state, registry, pin, frame, begun) =
        try_chosen(chosen_square(), (64, 18), false, &[0, 1]);
    begun.unwrap();
    assert_eq!(control_of(&state), Control::BoxMove);
    arm_capture(&mut state);
    drag(&mut state, &registry, &pin, &frame, (64, 27));
    let (points, _) = curve_points(&first_window(&state));
    assert!(near(fixed(&points[0].position.y), -0.125));
    assert!(near(fixed(&points[1].position.y), -0.125));
    assert!(near(left_key(&points[1].position.x), 0.2), "x unchanged");
    assert_eq!(
        points[1].out,
        WindowPoint::new(0.1, 0.05),
        "handles go with it"
    );
    assert_eq!(points[2], curve_points(&chosen_square()).0[2]);
    state.window_end("up");
    state.history.undo("desktop").unwrap().unwrap();
    assert_eq!(first_window(&state), chosen_square(), "one step");
    // A corner scales them uniformly about the centre; handles by the same factor.
    let (mut state, registry, pin, frame, begun) =
        try_chosen(chosen_square(), (89, 25), false, &[0, 1]);
    begun.unwrap();
    assert_eq!(control_of(&state), Control::BoxCorner(2));
    arm_capture(&mut state);
    drag(&mut state, &registry, &pin, &frame, (114, 33));
    // The handle follows the pointer along its down offset `u` from the centre: of that offset,
    // the box's 8 px off its corner (`inset` along `u`) does not scale.
    let (u, v) = ((25.5_f64, 7.5_f64), (50.5, 15.5));
    let from = u.0.hypot(u.1);
    let inset = 8.0 * (u.0 + u.1) / from;
    let k = 1.0 + ((v.0 * u.0 + v.1 * u.1) / from - from) / (from - inset);
    let (points, _) = curve_points(&first_window(&state));
    assert!(near(fixed(&points[0].position.x), -0.25 * k));
    assert!(near(fixed(&points[0].position.y), -0.25));
    // Anchor 1 was shown at .25 but its left key is .2: the key moves by the same amount.
    assert!(near(
        left_key(&points[1].position.x),
        0.2 + 0.25 * (k - 1.0)
    ));
    assert!(near(fixed(&points[1].out.x), 0.1 * k) && near(fixed(&points[1].out.y), 0.05 * k));
    assert!(near(fixed(&points[1].into.x), -0.05 * k));
    assert_eq!(points[3], curve_points(&chosen_square()).0[3], "unselected");
    // A side stretches them along its axis only.
    let (mut state, registry, pin, frame, begun) =
        try_chosen(chosen_square(), (89, 17), false, &[0, 1]);
    begun.unwrap();
    assert_eq!(control_of(&state), Control::BoxEdge(1));
    arm_capture(&mut state);
    drag(&mut state, &registry, &pin, &frame, (102, 30));
    let k = (38.5 - 8.0) / (25.5 - 8.0);
    let (points, _) = curve_points(&first_window(&state));
    assert!(near(fixed(&points[0].position.x), -0.25 * k));
    assert!(near(fixed(&points[0].position.y), -0.25));
    assert!(near(fixed(&points[1].out.x), 0.1 * k) && near(fixed(&points[1].out.y), 0.05));
    // Just beside a corner rotates them about the centre: a quarter turn, clockwise on screen.
    let (mut state, registry, pin, frame, begun) =
        try_chosen(chosen_square(), (96, 4), false, &[0, 1]);
    begun.unwrap();
    assert_eq!(control_of(&state), Control::BoxRotate(1));
    arm_capture(&mut state);
    drag(&mut state, &registry, &pin, &frame, (77, 50));
    let (points, _) = curve_points(&first_window(&state));
    assert!(near(fixed(&points[0].position.x), 0.0));
    assert!(near(fixed(&points[0].position.y), -0.5));
    assert!(near(left_key(&points[1].position.x), 0.2 - 0.25));
    assert!(near(fixed(&points[1].position.y), 0.0));
    assert!(near(fixed(&points[1].out.x), -0.05) && near(fixed(&points[1].out.y), 0.1));
    // Esc puts the whole list back, with no step; the point selection stays.
    let capture = state.window.active.take().unwrap();
    state.window_cancel(7, capture);
    assert_eq!(first_window(&state), chosen_square());
    assert!(!state.history.can_undo());
    assert_eq!(chosen_points(&state), [0, 1]);
}

#[test]
fn backspace_deletes_the_selected_anchors_in_one_step_and_esc_only_deselects_them() {
    let (mut state, _, _, _, begun) = try_chosen(chosen_square(), (64, 18), false, &[0, 1]);
    begun.unwrap();
    // Not during a capture.
    assert_eq!(
        state.window_points(7, 1, true).unwrap_err().data,
        Some(json!({"reason":"capture"}))
    );
    state.window_end("up");
    // Leaving two of four, both removed from the whole animation, as one step.
    let answer = state.window_points(7, 1, true).unwrap();
    assert_eq!(
        (
            answer["phase"].clone(),
            answer["points"].clone(),
            answer["selected"].clone()
        ),
        (json!("deleted"), json!([0, 1]), json!("w1"))
    );
    let (points, closed) = curve_points(&first_window(&state));
    assert_eq!(points[..], curve_points(&chosen_square()).0[2..]);
    assert!(closed);
    assert_eq!(chosen_points(&state), Vec::<u32>::new());
    assert_eq!(state.history.revision(), 1);
    state.history.undo("desktop").unwrap().unwrap();
    assert_eq!(first_window(&state), chosen_square());
    // Leaving one of a closed Curve refuses the whole delete and keeps the selection.
    let (mut state, _, _, _, begun) = try_chosen(chosen_square(), (64, 18), false, &[0, 1, 2]);
    begun.unwrap();
    state.window_end("up");
    let before = state.history.project().clone();
    assert_eq!(
        state.window_points(7, 1, true).unwrap_err().data,
        Some(json!({"reason":"delete","topology":{"why":"minimum","least":2,"shape":"curve"}}))
    );
    assert_eq!(state.history.project(), &before);
    assert_eq!(chosen_points(&state), [0, 1, 2]);
    // Esc clears the point selection (no step); with none it says so and Backspace is refused.
    let answer = state.window_points(7, 1, false).unwrap();
    assert_eq!(answer["cleared"], 3);
    assert_eq!(chosen_points(&state), Vec::<u32>::new());
    assert_eq!(state.window_points(7, 1, false).unwrap()["cleared"], 0);
    assert_eq!(
        state.window_points(7, 1, true).unwrap_err().data,
        Some(json!({"reason":"points"}))
    );
    assert_eq!(state.history.project(), &before);
    assert!(!state.history.can_undo());
    // Another connection's binding cannot.
    assert_eq!(
        state.window_points(8, 1, false).unwrap_err().data,
        Some(json!({"reason":"binding"}))
    );
}

#[test]
fn a_point_selection_outlives_its_drags_and_undo_but_not_another_down_or_anchor_count() {
    let (mut state, registry, pin, frame, begun) =
        try_chosen(chosen_square(), (64, 18), false, &[0, 1]);
    begun.unwrap();
    arm_capture(&mut state);
    drag(&mut state, &registry, &pin, &frame, (64, 27));
    state.window_end("up");
    assert_eq!(chosen_points(&state), [0, 1], "its own drag keeps it");
    state.history.undo("desktop").unwrap().unwrap();
    state.window_project_changed();
    assert_eq!(chosen_points(&state), [0, 1], "same anchor count");
    // A middle tap looks through the box to the anchor under it; the changed anchor count
    // clears the selection.
    let (mut state, registry, pin, frame, begun) =
        try_chosen(chosen_square(), (64, 18), false, &[0, 1]);
    begun.unwrap();
    state.window_end("up");
    let tap = middle_tap(&registry, &pin, &frame, (64, 18), false);
    assert_eq!(
        state.window_delete(7, 1, &tap).unwrap_err().data,
        Some(json!({"reason":"hit"})),
        "an edge under the box is not an anchor"
    );
    let tap = middle_tap(&registry, &pin, &frame, (46, 18), false);
    assert_eq!(state.window_delete(7, 1, &tap).unwrap()["point"], 0);
    assert_eq!(chosen_points(&state), Vec::<u32>::new());
    // A plain down outside the box, here on an unselected anchor, clears it and moves that one.
    let (state, _, _, _, begun) = try_chosen(chosen_square(), (82, 54), false, &[0, 1]);
    begun.unwrap();
    assert_eq!(control_of(&state), Control::Point(2));
    assert_eq!(chosen_points(&state), Vec::<u32>::new());
    // A down whose frame shows a different point selection is stale evidence.
    let (mut state, registry, pin, frame, begun) =
        try_chosen(chosen_square(), (64, 18), false, &[0, 1]);
    begun.unwrap();
    state.window_end("up");
    Arc::make_mut(state.window.selection.as_mut().unwrap()).chosen = None;
    let event = registry.input(Monitor::Timeline, 7, 1, pin.down.pointer, Some(frame));
    let old_down = registry.claim(7, event).unwrap();
    assert_eq!(
        state
            .window_begin(7, 1, old_down, registry.clone())
            .unwrap_err()
            .data,
        Some(json!({"reason":"selection"}))
    );
}

/// The canvas positions of `window`'s fixed anchors through the down frame's map of `w1` (box
/// drags change no transform, so that map still places them).
fn shown_anchors(frame: &FrameSnapshot, window: &Window) -> Vec<[f64; 2]> {
    let overlay = frame.overlay.as_ref().unwrap();
    let a = overlay
        .shapes
        .iter()
        .find(|s| s.id == "w1")
        .unwrap()
        .to_canvas;
    curve_points(window)
        .0
        .iter()
        .map(|p| {
            let (x, y) = (fixed(&p.position.x), fixed(&p.position.y));
            [a.xx * x + a.xy * y + a.tx, a.yx * x + a.yy * y + a.ty]
        })
        .collect()
}

/// The canvas box `[x0, y0, x1, y1]` of anchors 0 and 1.
fn first_two_bounds(anchors: &[[f64; 2]]) -> [f64; 4] {
    let (p, q) = (anchors[0], anchors[1]);
    [
        p[0].min(q[0]),
        p[1].min(q[1]),
        p[0].max(q[0]),
        p[1].max(q[1]),
    ]
}

/// The native pixel holding a canvas point (one device pixel per canvas pixel).
#[allow(
    clippy::cast_possible_truncation,
    reason = "device pixels near the surface"
)]
fn pixel(p: [f64; 2]) -> (i32, i32) {
    (p[0].floor() as i32, p[1].floor() as i32)
}

/// The fixture square as a Curve widened (Aspect 70) and turned 30 degrees: its local units are
/// neither square nor upright on the canvas.
fn turned_square() -> Window {
    let mut window = curve_window(&curve_square(), true);
    window.transform.aspect = Param::Fixed(70.0);
    window.transform.rotate = Param::Fixed(30.0);
    window
}

/// The turned square's anchors 0 and 1 on the canvas, their box and its centre.
fn turned_pair() -> (Vec<[f64; 2]>, [f64; 4], [f64; 2]) {
    let (_, _, _, frame, _) = try_chosen(turned_square(), (0, 0), false, &[0, 1]);
    let before = shown_anchors(&frame, &turned_square());
    let b = first_two_bounds(&before);
    (before, b, [b[0].midpoint(b[2]), b[1].midpoint(b[3])])
}

#[test]
fn box_rotation_is_rigid_on_screen_on_a_widened_turned_curve() {
    let near = |a: f64, b: f64| (a - b).abs() < 1e-9;
    let pad = columbina_view::window_overlay::BOX_PAD;
    // Just beside the box's corner 2 (right, bottom), outside it: a rotation about the centre.
    let (before, b, c) = turned_pair();
    let at = pixel([b[2] + pad + 6.0, b[3] + pad + 6.0]);
    let (mut state, registry, pin, frame, begun) = try_chosen(turned_square(), at, false, &[0, 1]);
    begun.unwrap();
    assert_eq!(control_of(&state), Control::BoxRotate(2));
    arm_capture(&mut state);
    let to = (70, 68);
    drag(&mut state, &registry, &pin, &frame, to);
    let after = shown_anchors(&frame, &first_window(&state));
    let u = (f64::from(at.0) + 0.5 - c[0], f64::from(at.1) + 0.5 - c[1]);
    let v = (f64::from(to.0) + 0.5 - c[0], f64::from(to.1) + 0.5 - c[1]);
    let turn = v.1.atan2(v.0) - u.1.atan2(u.0);
    let span = |p: &[[f64; 2]]| (p[1][0] - p[0][0], p[1][1] - p[0][1]);
    let (was, now) = (span(&before), span(&after));
    // Rigid on screen: the anchors keep their canvas distance and their distance from the
    // centre, and the line between them turns by the pointer's angle.
    assert!(
        near(was.0.hypot(was.1), now.0.hypot(now.1)),
        "{was:?} {now:?}"
    );
    for k in 0..2 {
        assert!(near(
            (before[k][0] - c[0]).hypot(before[k][1] - c[1]),
            (after[k][0] - c[0]).hypot(after[k][1] - c[1])
        ));
    }
    let off = (now.1.atan2(now.0) - was.1.atan2(was.0) - turn).rem_euclid(std::f64::consts::TAU);
    assert!(near(off, 0.0) || near(off, std::f64::consts::TAU), "{off}");
    assert_eq!(after[2..], before[2..], "unselected");
}

#[test]
fn a_box_side_stretches_one_canvas_axis_on_a_widened_turned_curve() {
    let near = |a: f64, b: f64| (a - b).abs() < 1e-9;
    let pad = columbina_view::window_overlay::BOX_PAD;
    // The right side stretches them along the canvas x axis only, by one factor.
    let (before, b, c) = turned_pair();
    let at = pixel([b[2] + pad, c[1]]);
    let (mut state, registry, pin, frame, begun) = try_chosen(turned_square(), at, false, &[0, 1]);
    begun.unwrap();
    assert_eq!(control_of(&state), Control::BoxEdge(1));
    arm_capture(&mut state);
    drag(&mut state, &registry, &pin, &frame, (at.0 + 12, at.1 + 5));
    let after = shown_anchors(&frame, &first_window(&state));
    let k = (after[0][0] - c[0]) / (before[0][0] - c[0]);
    assert!(k > 1.0, "{k}");
    for i in 0..2 {
        assert!(near(after[i][1], before[i][1]), "canvas y kept: {after:?}");
        assert!(near(after[i][0] - c[0], k * (before[i][0] - c[0])));
    }
}

/// Anchors 0 and 1 three device pixels apart on each axis (canvas 62.5, 16.5 and 65.5, 19.5;
/// centre 64, 18), 2 and 3 as the fixture square's.
fn close_pair() -> Window {
    let local = |x: f64, y: f64| [(x - 64.0) / 72.0, (y - 36.0) / 72.0];
    curve_window(
        &[
            (local(62.5, 16.5), [0.0; 2], [0.0; 2]),
            (local(65.5, 19.5), [0.0; 2], [0.0; 2]),
            (local(82.0, 54.0), [0.0; 2], [0.0; 2]),
            (local(46.0, 54.0), [0.0; 2], [0.0; 2]),
        ],
        true,
    )
}

#[test]
fn a_box_handle_stays_under_the_pointer_however_close_the_selected_anchors() {
    let near = |a: f64, b: f64| (a - b).abs() < 1e-9;
    let pad = columbina_view::window_overlay::BOX_PAD;
    // The right side's handle is at canvas (64 + 1.5 + 8, 18): native pixel (73, 17) holds it.
    let (mut state, registry, pin, frame, begun) =
        try_chosen(close_pair(), (73, 17), false, &[0, 1]);
    begun.unwrap();
    assert_eq!(control_of(&state), Control::BoxEdge(1));
    arm_capture(&mut state);
    drag(&mut state, &registry, &pin, &frame, (93, 17));
    let b = first_two_bounds(&shown_anchors(&frame, &first_window(&state)));
    assert!(
        near(b[2] + pad, 93.5),
        "side at {} for the pointer at 93.5",
        b[2] + pad
    );
    assert!(near(b[1], 16.5) && near(b[3], 19.5), "y kept");
    // The corner 2 handle, at canvas (73.5, 27.5), dragged three times as far from the centre.
    let (mut state, registry, pin, frame, begun) =
        try_chosen(close_pair(), (73, 27), false, &[0, 1]);
    begun.unwrap();
    assert_eq!(control_of(&state), Control::BoxCorner(2));
    arm_capture(&mut state);
    drag(&mut state, &registry, &pin, &frame, (92, 46));
    let b = first_two_bounds(&shown_anchors(&frame, &first_window(&state)));
    assert!(
        near(b[2] + pad, 92.5) && near(b[3] + pad, 46.5),
        "corner at {:?} for the pointer at (92.5, 46.5)",
        (b[2] + pad, b[3] + pad)
    );
}

#[test]
fn a_project_change_that_drops_the_point_selection_tells_its_owner() {
    let (mut state, _, _, _, begun) = try_chosen(chosen_square(), (64, 18), false, &[0, 1]);
    begun.unwrap();
    state.window_end("up");
    let binding = state.window.selection.as_ref().unwrap().binding;
    let (mut owner, _) = binding_peer(&mut state, 7);
    let (mut observer, _) = binding_peer(&mut state, 8);
    // A change keeping the Curve's anchor count keeps the selection and says nothing.
    state
        .apply_worked_out_locked(Kind::Agent, None, None, Instant::now(), |_| {
            Ok(vec![
                json!({"op":"set_track","timeline":"t1","id":"V1","name":"still valid"}),
            ])
        })
        .unwrap();
    assert_eq!(chosen_points(&state), [0, 1]);
    assert_eq!(window_notifications(&mut owner), Vec::<Value>::new());
    // One changing it (another client's edit here; an undo or redo alike) drops the selection
    // and tells its owner, so that the desktop's count follows.
    let selection = state.window.selection.clone().unwrap();
    let mut windows = selection
        .corrector(state.history.project())
        .unwrap()
        .windows
        .clone();
    let WindowShape::Curve { points, .. } = &mut windows[0].shape else {
        panic!()
    };
    points.pop();
    let op = Movement::op(&selection, &windows);
    state
        .apply_worked_out_locked(Kind::Agent, None, None, Instant::now(), |_| Ok(vec![op]))
        .unwrap();
    assert_eq!(chosen_points(&state), Vec::<u32>::new());
    let notes = window_notifications(&mut owner);
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert_eq!(notes[0]["method"], "window.points");
    assert_eq!(
        notes[0]["params"],
        json!({"binding":binding,"sequence":state.window.sequence,
            "revision":state.history.revision(),"selected_points":[]})
    );
    // With none selected, later changes say nothing; nobody else is told.
    state
        .apply_worked_out_locked(Kind::Agent, None, None, Instant::now(), |_| {
            Ok(vec![
                json!({"op":"set_track","timeline":"t1","id":"V1","name":"again"}),
            ])
        })
        .unwrap();
    assert_eq!(window_notifications(&mut owner), Vec::<Value>::new());
    assert_eq!(window_notifications(&mut observer), Vec::<Value>::new());
}

/// Frame mode keys a drag of a window never tracked (T24, p. 3250): its tracker is set up on
/// the frames a track of its clip (1920x1080, so 1280x720) would have, anchored at its centre,
/// and the move becomes the tracker's keyframe at the playhead, its Transform kept.
#[test]
fn frame_mode_keys_a_move_of_a_window_never_tracked() {
    use columbina_model::grade::window::tracking::{Mode, WindowTracking};
    let mut window = Window::new("w1", WindowShape::circular());
    window.tracking = Some(WindowTracking {
        mode: Mode::Frame,
        ..WindowTracking::default()
    });
    let (mut state, registry, pin, frame) = control_window_fixture(window, (64, 36));
    arm(&mut state);
    drag(&mut state, &registry, &pin, &frame, (80, 36));
    let changed = first_window(&state);
    assert_eq!(changed.transform.pan, Param::Fixed(50.0));
    let data = &changed.tracking.unwrap().data;
    assert_eq!(data.frame_size, [1280, 720]);
    assert_eq!(data.anchor, [639.5, 359.5]);
    assert_eq!(data.keys().keys().copied().collect::<Vec<_>>(), vec![10]);
    // 16 of the view's 128 pixels: an eighth of the frame, 160 of its 1280 pixels.
    assert!(
        (data.key_at(10).pan - 160.0).abs() < 1e-9,
        "{:?}",
        data.key_at(10)
    );
    assert!(data.key_at(10).tilt.abs() < 1e-9);
}
