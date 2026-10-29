use core::fmt::Debug;
use core::mem;

#[derive(Clone, Debug)]
pub enum Cached<T: Clone + Debug> {
    Empty,
    Unused(T),
    Used(T),
}

impl<T: Clone + Debug> Cached<T> {
    pub const fn get(&self) -> Option<&T> {
        match self {
            Self::Empty | Self::Unused(_) => None,
            Self::Used(t) => Some(t),
        }
    }

    pub fn get_mut(&mut self) -> Option<&mut T> {
        match self {
            Self::Empty | Self::Unused(_) => None,
            Self::Used(t) => Some(t),
        }
    }

    pub const fn is_unused(&self) -> bool {
        match self {
            Self::Empty | Self::Unused(_) => true,
            Self::Used(_) => false,
        }
    }

    pub const fn is_used(&self) -> bool {
        match self {
            Self::Empty | Self::Unused(_) => false,
            Self::Used(_) => true,
        }
    }

    pub const fn is_invalidated(&self) -> bool {
        matches!(self, Self::Unused(_))
    }

    pub fn take_unused(&mut self) -> Option<T> {
        if matches!(*self, Self::Unused(_)) {
            let Self::Unused(val) = mem::replace(self, Self::Empty) else {
                unreachable!()
            };
            Some(val)
        } else {
            None
        }
    }

    pub fn take_used(&mut self) -> Option<T> {
        if matches!(*self, Self::Used(_)) {
            let Self::Used(val) = mem::replace(self, Self::Empty) else {
                unreachable!()
            };
            Some(val)
        } else {
            None
        }
    }

    #[allow(clippy::missing_panics_doc)]
    pub fn set_unused(&mut self) {
        if matches!(*self, Self::Used(_)) {
            *self = Self::Unused(self.take_used().expect("cached value should be used"));
        }
    }

    pub fn set_used(&mut self, val: T) {
        *self = Self::Used(val);
    }
}
