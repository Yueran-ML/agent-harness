use super::*;
use crate::presentation::{LayerSnapshot, Origin};
use crate::session::GESTURE_IDLE;
use columbina_model::grade::{Interp, Key, Node, NodeKind, Param, Target, Window, WindowShape};
use columbina_model::{History, Op};

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
            assert_eq!(viewer.test_pointer_owner(Monitor::Timeline), Some(7),
                "a later bind cannot change the pointer while the earlier turn is waiting for State");
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
                enabled: None,
                locked: None,
                language: None,
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
    assert!((overlay.circles[0].to_canvas.tx - evaluated * 1.28).abs() < 1e-10);
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
    let a = overlay.circles[0].to_canvas;
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
