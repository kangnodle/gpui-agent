use super::{Adapter,
            State};
use crate::{LPARAM,
            WPARAM};
use accesskit::{ActionHandler,
                ActionRequest,
                ActivationHandler,
                Node,
                NodeId,
                Rect,
                Role,
                Tree,
                TreeId,
                TreeUpdate};
use windows::{Win32::UI::{Accessibility::*,
                          WindowsAndMessaging::*},
              core::w};

struct Handler;
impl ActionHandler for Handler {
    fn do_action(&mut self, _: ActionRequest) {}
}
impl ActivationHandler for Handler {
    fn request_initial_tree(&mut self) -> Option<TreeUpdate> {
        let mut root = Node::new(Role::Window);
        root.set_bounds(Rect::new(0., 0., 100., 100.));
        root.set_children([NodeId(2)]);
        let mut input = Node::new(Role::TextInput);
        input.set_bounds(Rect::new(0., 0., 100., 20.));
        input.set_children([NodeId(3)]);
        let mut text = Node::new(Role::TextRun);
        text.set_value("text");
        text.set_character_lengths([1, 1, 1, 1]);
        text.set_character_positions([0., 10., 20., 30.]);
        text.set_character_widths([10., 10., 10., 10.]);
        text.set_text_direction(accesskit::TextDirection::LeftToRight);
        text.set_bounds(Rect::new(0., 0., 40., 20.));
        Some(TreeUpdate {
            nodes: vec![(NodeId(1), root), (NodeId(2), input), (NodeId(3), text)],
            tree: Some(Tree::new(NodeId(1))),
            tree_id: TreeId::ROOT,
            focus: NodeId(2),
        })
    }
}

struct Window(windows::Win32::Foundation::HWND);
impl Drop for Window {
    fn drop(&mut self) { let _ = unsafe { DestroyWindow(self.0) }; }
}

fn fixture() -> windows::core::Result<(Window, Adapter, IRawElementProviderFragment)> {
    let window = Window(unsafe {
        CreateWindowExW(
            Default::default(),
            w!("STATIC"),
            w!("AccessKit close regression"),
            WS_OVERLAPPEDWINDOW,
            180,
            140,
            200,
            180,
            None,
            None,
            None,
            None,
        )?
    });
    let mut adapter = Adapter::new(window.0, false, Handler);
    drop(adapter.handle_wm_getobject(WPARAM(0), LPARAM(UiaRootObjectId as isize), &mut Handler));
    let State::Active(context) = &adapter.state else {
        panic!("Adapter must be active")
    };
    let root = context.get_or_create_platform_node(context.read_tree().state().root_id()).into_interface();
    Ok((window, adapter, root))
}

#[test]
fn given_a_retained_provider_when_its_window_is_destroyed_then_bounds_are_unavailable_without_panicking() -> windows::core::Result<()> {
    unsafe {
        use windows::core::Interface;
        let (window, _adapter, fragment) = fixture()?;
        let bounds = fragment.BoundingRectangle()?;
        assert_eq!((bounds.width, bounds.height), (100., 100.));
        assert!(bounds.left >= 180. && bounds.top >= 140.);
        let root: IRawElementProviderFragmentRoot = fragment.cast()?;
        assert!(root.ElementProviderFromPoint(bounds.left + 1., bounds.top + 1.).is_ok());
        DestroyWindow(window.0)?;
        assert_eq!(fragment.BoundingRectangle().unwrap_err().code().0, UIA_E_ELEMENTNOTAVAILABLE as i32);
        assert_eq!(
            root.ElementProviderFromPoint(bounds.left + 1., bounds.top + 1.).unwrap_err().code().0,
            UIA_E_ELEMENTNOTAVAILABLE as i32
        );
        Ok(())
    }
}

#[test]
fn given_retained_text_when_its_window_is_destroyed_then_text_geometry_is_unavailable_without_panicking() -> windows::core::Result<()> {
    use windows::core::Interface;
    unsafe {
        let (window, _adapter, root) = fixture()?;
        let input = root.Navigate(NavigateDirection_FirstChild)?;
        let provider: ITextProvider = input.cast()?;
        let range = provider.DocumentRange()?;
        let bounds = input.BoundingRectangle()?;
        let rectangles = range.GetBoundingRectangles()?;
        assert!(!rectangles.is_null());
        windows::Win32::System::Ole::SafeArrayDestroy(rectangles)?;
        let point = UiaPoint {
            x: bounds.left + 1.,
            y: bounds.top + 1.,
        };
        assert!(provider.RangeFromPoint(point).is_ok());
        DestroyWindow(window.0)?;
        assert_eq!(range.GetBoundingRectangles().unwrap_err().code().0, UIA_E_ELEMENTNOTAVAILABLE as i32);
        assert_eq!(provider.RangeFromPoint(point).unwrap_err().code().0, UIA_E_ELEMENTNOTAVAILABLE as i32);
        Ok(())
    }
}

#[test]
fn given_a_released_adapter_when_a_client_retains_the_provider_then_bounds_are_unavailable() -> windows::core::Result<()> {
    let (_window, adapter, fragment) = fixture()?;
    assert!(unsafe { fragment.BoundingRectangle() }.is_ok());
    drop(adapter);
    assert_eq!(unsafe { fragment.BoundingRectangle() }.unwrap_err().code().0, UIA_E_ELEMENTNOTAVAILABLE as i32);
    Ok(())
}
