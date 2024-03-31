use std::{
    mem::ManuallyDrop,
    rc::Rc,
    task::{RawWaker, RawWakerVTable, Waker},
};

use crate::task::Task;

pub fn waker(task: Rc<Task>) -> Waker {
    let raw = Rc::into_raw(task).cast::<()>();
    let vtable = &WakerHelper::VTABLE;
    unsafe { Waker::from_raw(RawWaker::new(raw, vtable)) }
}

struct WakerHelper;

impl WakerHelper {
    const VTABLE: RawWakerVTable =
        RawWakerVTable::new(Self::clone, Self::wake, Self::wake_by_ref, Self::drop);

    unsafe fn clone(ptr: *const ()) -> RawWaker {
        let rc = Rc::from_raw(ptr.cast::<Task>());
        std::mem::forget(Rc::clone(&rc));
        RawWaker::new(ptr, &Self::VTABLE)
    }

    unsafe fn wake(ptr: *const ()) {
        let rc = Rc::from_raw(ptr.cast::<Task>());
        rc.schedule();
    }

    unsafe fn wake_by_ref(ptr: *const ()) {
        let rc = ManuallyDrop::new(Rc::from_raw(ptr.cast::<Task>()));
        rc.schedule();
    }

    unsafe fn drop(ptr: *const ()) {
        drop(Rc::from_raw(ptr.cast::<Task>()));
    }
}
