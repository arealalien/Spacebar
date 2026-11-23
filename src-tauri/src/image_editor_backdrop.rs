use std::{
    cell::{OnceCell, RefCell},
    mem::size_of,
};
use windows::{
    core::{Interface, Result},
    System::{DispatcherQueue, DispatcherQueueController},
    Win32::{
        Foundation::HWND as DesktopHwnd,
        System::WinRT::{
            Composition::ICompositorDesktopInterop, CreateDispatcherQueueController,
            DispatcherQueueOptions, DQTAT_COM_NONE, DQTYPE_THREAD_CURRENT,
        },
    },
    UI::Composition::{
        CompositionRoundedRectangleGeometry, Compositor, Desktop::DesktopWindowTarget,
        SpriteVisual, Visual,
    },
};
use windows_numerics::Vector2;
use windows_sys::Win32::{
    Foundation::{HWND, RECT},
    Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_USE_HOSTBACKDROPBRUSH},
    UI::{
        HiDpi::GetDpiForWindow,
        WindowsAndMessaging::{GetWindowRect, IsIconic},
    },
};

// Match the corner radius of the crop editor's CSS.
const RADIUS: f32 = 26.0;

struct Frame {
    bounds: RECT,
    radius: f32,
}

impl Frame {
    fn read(hwnd: HWND) -> Option<Self> {
        let mut bounds = RECT::default();
        unsafe {
            if IsIconic(hwnd) != 0
                || GetWindowRect(hwnd, &mut bounds) == 0
                || bounds.right <= bounds.left
                || bounds.bottom <= bounds.top
            {
                return None;
            }
            let dpi = GetDpiForWindow(hwnd);
            let radius = if dpi == 0 {
                RADIUS
            } else {
                RADIUS * dpi as f32 / 96.0
            };
            Some(Self { bounds, radius })
        }
    }
}

struct Backdrop {
    hwnd: HWND,
    _compositor: Compositor,
    target: DesktopWindowTarget,
    visual: SpriteVisual,
    clip: CompositionRoundedRectangleGeometry,
}

impl Backdrop {
    fn resize(&self, frame: &Frame) -> Result<()> {
        let size = Vector2 {
            X: (frame.bounds.right - frame.bounds.left) as f32,
            Y: (frame.bounds.bottom - frame.bounds.top) as f32,
        };
        self.clip.SetCornerRadius(Vector2 {
            X: frame.radius,
            Y: frame.radius,
        })?;
        self.clip.SetSize(size)?;
        self.visual.SetSize(size)
    }
}

// The dispatcher and composition objects belong to the native window thread.
thread_local! {
    static QUEUE: OnceCell<DispatcherQueueController> = const { OnceCell::new() };
    static BACKDROP: RefCell<Option<Backdrop>> = const { RefCell::new(None) };
}

pub(crate) fn attach(hwnd: HWND) -> bool {
    let Some(frame) = Frame::read(hwnd) else {
        return false;
    };
    create_backdrop(hwnd, &frame).is_ok()
}

fn create_backdrop(hwnd: HWND, frame: &Frame) -> Result<()> {
    let enabled = 1i32;
    windows_core::HRESULT(unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_HOSTBACKDROPBRUSH as u32,
            (&enabled as *const i32).cast(),
            size_of::<i32>() as u32,
        )
    })
    .ok()?;

    if DispatcherQueue::GetForCurrentThread().is_err() {
        let queue = unsafe {
            CreateDispatcherQueueController(DispatcherQueueOptions {
                dwSize: size_of::<DispatcherQueueOptions>() as u32,
                threadType: DQTYPE_THREAD_CURRENT,
                apartmentType: DQTAT_COM_NONE,
            })?
        };
        QUEUE.with(|current| {
            let _ = current.set(queue);
        });
    }

    let compositor = Compositor::new()?;
    let interop: ICompositorDesktopInterop = compositor.cast()?;
    let target = unsafe { interop.CreateDesktopWindowTarget(DesktopHwnd(hwnd), false)? };
    let visual = compositor.CreateSpriteVisual()?;
    let clip = compositor.CreateRoundedRectangleGeometry()?;
    // Clip the visual itself to keep the desktop blur within the editor's corners.
    visual.SetClip(&compositor.CreateGeometricClipWithGeometry(&clip)?)?;
    visual.SetBrush(&compositor.CreateHostBackdropBrush()?)?;
    let backdrop = Backdrop {
        hwnd,
        _compositor: compositor,
        target,
        visual,
        clip,
    };
    backdrop.resize(frame)?;
    backdrop.target.SetRoot(&backdrop.visual)?;
    BACKDROP.replace(Some(backdrop));
    Ok(())
}

pub(crate) fn update_frame(hwnd: HWND) -> Result<()> {
    let Some(frame) = Frame::read(hwnd) else {
        return Ok(());
    };
    crate::system_panel::set_panel_region(
        hwnd,
        frame.bounds,
        (frame.radius * 2.0).round() as i32,
        true,
    );
    BACKDROP.with_borrow(|current| {
        current
            .as_ref()
            .filter(|backdrop| backdrop.hwnd == hwnd)
            .map_or(Ok(()), |backdrop| backdrop.resize(&frame))
    })
}

pub(crate) fn detach(hwnd: HWND) {
    let backdrop = BACKDROP.with_borrow_mut(|current| {
        if current
            .as_ref()
            .is_some_and(|backdrop| backdrop.hwnd == hwnd)
        {
            current.take()
        } else {
            None
        }
    });
    if let Some(backdrop) = backdrop {
        let _ = backdrop.target.SetRoot(None::<&Visual>);
    }
}
