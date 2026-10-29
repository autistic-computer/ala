use crate::{Action, App, Platform, SharedString};

pub struct Menu {
    pub name: SharedString,

    pub items: Vec<MenuItem>,

    pub disabled: bool,
}

impl Menu {
    pub fn new(name: impl Into<SharedString>) -> Self {
        Self {
            name: name.into(),
            items: vec![],
            disabled: false,
        }
    }

    pub fn items(mut self, items: impl IntoIterator<Item = MenuItem>) -> Self {
        self.items = items.into_iter().collect();
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn owned(self) -> OwnedMenu {
        OwnedMenu {
            name: self.name.to_string().into(),
            items: self.items.into_iter().map(|item| item.owned()).collect(),
            disabled: self.disabled,
        }
    }
}

pub struct OsMenu {
    pub name: SharedString,

    pub menu_type: SystemMenuType,
}

impl OsMenu {
    pub fn owned(self) -> OwnedOsMenu {
        OwnedOsMenu {
            name: self.name.to_string().into(),
            menu_type: self.menu_type,
        }
    }
}

#[derive(Copy, Clone, Eq, PartialEq)]
pub enum SystemMenuType {
    Services,
}

pub enum MenuItem {
    Separator,

    Submenu(Menu),

    SystemMenu(OsMenu),

    Action {
        name: SharedString,

        action: Box<dyn Action>,

        os_action: Option<OsAction>,

        checked: bool,

        disabled: bool,
    },
}

impl MenuItem {
    pub fn separator() -> Self {
        Self::Separator
    }

    pub fn submenu(menu: Menu) -> Self {
        Self::Submenu(menu)
    }

    pub fn os_submenu(name: impl Into<SharedString>, menu_type: SystemMenuType) -> Self {
        Self::SystemMenu(OsMenu {
            name: name.into(),
            menu_type,
        })
    }

    pub fn action(name: impl Into<SharedString>, action: impl Action) -> Self {
        Self::Action {
            name: name.into(),
            action: Box::new(action),
            os_action: None,
            checked: false,
            disabled: false,
        }
    }

    pub fn os_action(
        name: impl Into<SharedString>,
        action: impl Action,
        os_action: OsAction,
    ) -> Self {
        Self::Action {
            name: name.into(),
            action: Box::new(action),
            os_action: Some(os_action),
            checked: false,
            disabled: false,
        }
    }

    pub fn owned(self) -> OwnedMenuItem {
        match self {
            MenuItem::Separator => OwnedMenuItem::Separator,
            MenuItem::Submenu(submenu) => OwnedMenuItem::Submenu(submenu.owned()),
            MenuItem::Action {
                name,
                action,
                os_action,
                checked,
                disabled,
            } => OwnedMenuItem::Action {
                name: name.into(),
                action,
                os_action,
                checked,
                disabled,
            },
            MenuItem::SystemMenu(os_menu) => OwnedMenuItem::SystemMenu(os_menu.owned()),
        }
    }

    pub fn checked(mut self, checked: bool) -> Self {
        match &mut self {
            MenuItem::Action { checked: old, .. } => {
                *old = checked;
            }
            _ => {}
        }
        self
    }

    #[inline]
    pub fn is_checked(&self) -> bool {
        match self {
            MenuItem::Action { checked, .. } => *checked,
            _ => false,
        }
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        match &mut self {
            MenuItem::Action { disabled: old, .. } => {
                *old = disabled;
            }
            MenuItem::Submenu(submenu) => {
                submenu.disabled = disabled;
            }
            _ => {}
        }
        self
    }

    #[inline]
    pub fn is_disabled(&self) -> bool {
        match self {
            MenuItem::Action { disabled, .. } => *disabled,
            MenuItem::Submenu(submenu) => submenu.disabled,
            _ => false,
        }
    }
}

#[derive(Clone)]
pub struct OwnedOsMenu {
    pub name: SharedString,

    pub menu_type: SystemMenuType,
}

#[derive(Clone)]
pub struct OwnedMenu {
    pub name: SharedString,

    pub items: Vec<OwnedMenuItem>,

    pub disabled: bool,
}

pub enum OwnedMenuItem {
    Separator,

    Submenu(OwnedMenu),

    SystemMenu(OwnedOsMenu),

    Action {
        name: String,

        action: Box<dyn Action>,

        os_action: Option<OsAction>,

        checked: bool,

        disabled: bool,
    },
}

impl Clone for OwnedMenuItem {
    fn clone(&self) -> Self {
        match self {
            OwnedMenuItem::Separator => OwnedMenuItem::Separator,
            OwnedMenuItem::Submenu(submenu) => OwnedMenuItem::Submenu(submenu.clone()),
            OwnedMenuItem::Action {
                name,
                action,
                os_action,
                checked,
                disabled,
            } => OwnedMenuItem::Action {
                name: name.clone(),
                action: action.boxed_clone(),
                os_action: *os_action,
                checked: *checked,
                disabled: *disabled,
            },
            OwnedMenuItem::SystemMenu(os_menu) => OwnedMenuItem::SystemMenu(os_menu.clone()),
        }
    }
}

#[derive(Copy, Clone, Eq, PartialEq)]
pub enum OsAction {
    Cut,

    Copy,

    Paste,

    SelectAll,

    Undo,

    Redo,
}

pub(crate) fn init_app_menus(platform: &dyn Platform, cx: &App) {
    platform.on_will_open_app_menu(Box::new({
        let cx = cx.to_async();
        move || {
            if let Some(app) = cx.app.upgrade() {
                app.borrow_mut().update(|cx| cx.clear_pending_keystrokes());
            }
        }
    }));

    platform.on_validate_app_menu_command(Box::new({
        let cx = cx.to_async();
        move |action| {
            cx.app
                .upgrade()
                .map(|app| app.borrow_mut().update(|cx| cx.is_action_available(action)))
                .unwrap_or(false)
        }
    }));

    platform.on_app_menu_action(Box::new({
        let cx = cx.to_async();
        move |action| {
            if let Some(app) = cx.app.upgrade() {
                app.borrow_mut().update(|cx| cx.dispatch_action(action));
            }
        }
    }));
}

#[cfg(test)]
mod tests {
    use crate::Menu;

    #[test]
    fn test_menu() {
        let menu = Menu::new("App")
            .items(vec![
                crate::MenuItem::action("Action 1", ala::NoAction),
                crate::MenuItem::separator(),
            ])
            .disabled(true);

        assert_eq!(menu.name.as_ref(), "App");
        assert_eq!(menu.items.len(), 2);
        assert!(menu.disabled);
    }

    #[test]
    fn test_menu_item_builder() {
        use super::MenuItem;

        let item = MenuItem::action("Test Action", ala::NoAction);
        assert_eq!(
            match &item {
                MenuItem::Action { name, .. } => name.as_ref(),
                _ => unreachable!(),
            },
            "Test Action"
        );
        assert!(matches!(
            item,
            MenuItem::Action {
                checked: false,
                disabled: false,
                ..
            }
        ));

        assert!(
            MenuItem::action("Test Action", ala::NoAction)
                .checked(true)
                .is_checked()
        );
        assert!(
            MenuItem::action("Test Action", ala::NoAction)
                .disabled(true)
                .is_disabled()
        );

        let submenu = MenuItem::submenu(super::Menu {
            name: "Submenu".into(),
            items: vec![],
            disabled: true,
        });
        assert_eq!(
            match &submenu {
                MenuItem::Submenu(menu) => menu.name.as_ref(),
                _ => unreachable!(),
            },
            "Submenu"
        );
        assert!(!submenu.is_checked());
        assert!(submenu.is_disabled());
    }
}
