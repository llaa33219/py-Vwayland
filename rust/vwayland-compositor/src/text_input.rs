//! Compositor side of `zwp_text_input_v3` (the IME insertion path of
//! `type_text`).
//!
//! Smithay ships a `zwp_text_input_v3` module, but it is built around an
//! *external* IME client: its request handler drops every `enable`/`commit`
//! unless a `zwp_input_method_v1` instance is connected. Here the compositor
//! itself is the input method (nothing else talks to the app), so the text-input
//! objects are tracked here directly and `commit_string`/`done` are sent from
//! the compositor side, which is where those events belong.
//!
//! Only what `type_text` needs is implemented: enter/leave on focus change, the
//! double-buffered enable/disable state, and the `done` serial counter. The
//! state requests of the client (surrounding text, content type, cursor
//! rectangle) are accepted and ignored — the compositor does not act on them.

use std::collections::HashMap;

use smithay::reexports::wayland_protocols::wp::text_input::zv3::server::{
    zwp_text_input_manager_v3::{Request as ManagerRequest, ZwpTextInputManagerV3},
    zwp_text_input_v3::{Request as TextInputRequest, ZwpTextInputV3},
};
use smithay::reexports::wayland_server::backend::{ClientId, GlobalId, ObjectId};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};

use crate::state::Vwayland;

/// Version of the advertised `zwp_text_input_manager_v3` global.
const MANAGER_VERSION: u32 = 1;

/// Text-input objects of every connected client.
pub struct TextInputState {
    #[allow(dead_code)]
    global: GlobalId,
    instances: HashMap<ObjectId, Instance>,
    /// Surface the text-input focus currently sits on (`enter` was sent for it).
    focus: Option<WlSurface>,
}

struct Instance {
    resource: ZwpTextInputV3,
    /// `enable`/`disable` are double-buffered and applied by `commit`.
    pending_enable: Option<bool>,
    enabled: bool,
    /// Number of `commit` requests received; the protocol mandates this count
    /// as the serial of every `done` event.
    serial: u32,
}

impl TextInputState {
    pub fn new(display: &DisplayHandle) -> Self
    where
        Vwayland: GlobalDispatch<ZwpTextInputManagerV3, ()>
            + Dispatch<ZwpTextInputManagerV3, ()>
            + Dispatch<ZwpTextInputV3, ()>,
    {
        let global = display.create_global::<Vwayland, ZwpTextInputManagerV3, _>(MANAGER_VERSION, ());
        Self {
            global,
            instances: HashMap::new(),
            focus: None,
        }
    }

    fn add_instance(&mut self, resource: ZwpTextInputV3) {
        self.instances.insert(
            resource.id(),
            Instance {
                resource,
                pending_enable: None,
                enabled: false,
                serial: 0,
            },
        );
    }

    fn remove_instance(&mut self, id: &ObjectId) {
        self.instances.remove(id);
    }

    /// Buffer an `enable`/`disable` request until the next `commit`.
    fn set_pending_enable(&mut self, resource: &ZwpTextInputV3, enable: bool) {
        if let Some(instance) = self.instances.get_mut(&resource.id()) {
            instance.pending_enable = Some(enable);
        }
    }

    /// Apply the buffered state and acknowledge with `done`.
    fn commit(&mut self, resource: &ZwpTextInputV3) {
        let Some(instance) = self.instances.get_mut(&resource.id()) else {
            return;
        };
        instance.serial += 1;
        if let Some(enable) = instance.pending_enable.take() {
            instance.enabled = enable;
        }
        let serial = instance.serial;
        resource.done(serial);
    }

    /// Follow the keyboard focus: `leave` for the previous client, `enter` for
    /// the new one. Must be called on every focus change.
    pub fn focus_changed(&mut self, focused: Option<&WlSurface>) {
        let previous = self.focus.take();
        if let Some(surface) = previous.as_ref() {
            // The events are collected first: sending them borrows the resource,
            // which is stored in the map being iterated.
            let targets: Vec<ZwpTextInputV3> = self
                .instances
                .values()
                .filter(|i| i.resource.id().same_client_as(&surface.id()))
                .map(|i| i.resource.clone())
                .collect();
            for target in targets {
                target.leave(&surface);
            }
        }
        if let Some(surface) = focused {
            let targets: Vec<(ZwpTextInputV3, u32)> = self
                .instances
                .values()
                .filter(|i| i.resource.id().same_client_as(&surface.id()))
                .map(|i| (i.resource.clone(), i.serial))
                .collect();
            for (target, serial) in targets {
                target.enter(&surface);
                target.done(serial);
            }
        }
        self.focus = focused.cloned();
    }

    /// Insert `text` into the enabled text input of the focused client.
    ///
    /// Returns `false` when the client has no enabled text input, which is the
    /// caller's signal to fall back to key event typing.
    pub fn commit_text(&mut self, text: &str) -> bool {
        let Some(focus) = self.focus.as_ref() else {
            return false;
        };
        let target = self
            .instances
            .values()
            .find(|i| i.enabled && i.resource.id().same_client_as(&focus.id()))
            .map(|i| (i.resource.clone(), i.serial));
        let Some((resource, serial)) = target else {
            return false;
        };
        resource.commit_string(Some(text.to_string()));
        resource.done(serial);
        true
    }
}

impl Vwayland {
    /// Deliver `text` through `zwp_text_input_v3` (layer C of `type_text`).
    pub fn commit_text_input(&mut self, text: &str) -> bool {
        self.text_input_state.commit_text(text)
    }

    /// Re-run enter/leave after a client created a text input object while the
    /// keyboard focus is already on its surface.
    fn new_text_input_object(&mut self, resource: &ZwpTextInputV3) {
        self.text_input_state.add_instance(resource.clone());
        if let Some(focus) = self.text_input_state.focus.clone() {
            if resource.id().same_client_as(&focus.id()) {
                resource.enter(&focus);
                resource.done(0);
            }
        }
    }
}

impl GlobalDispatch<ZwpTextInputManagerV3, (), Vwayland> for Vwayland {
    fn bind(
        _state: &mut Vwayland,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<ZwpTextInputManagerV3>,
        _data: &(),
        data_init: &mut DataInit<'_, Vwayland>,
    ) {
        data_init.init(resource, ());
    }
}

impl Dispatch<ZwpTextInputManagerV3, (), Vwayland> for Vwayland {
    fn request(
        state: &mut Vwayland,
        _client: &Client,
        _resource: &ZwpTextInputManagerV3,
        request: ManagerRequest,
        _data: &(),
        _handle: &DisplayHandle,
        data_init: &mut DataInit<'_, Vwayland>,
    ) {
        match request {
            ManagerRequest::GetTextInput { id, .. } => {
                let instance = data_init.init(id, ());
                state.new_text_input_object(&instance);
            }
            ManagerRequest::Destroy => {}
            _ => {}
        }
    }
}

impl Dispatch<ZwpTextInputV3, (), Vwayland> for Vwayland {
    fn request(
        state: &mut Vwayland,
        _client: &Client,
        resource: &ZwpTextInputV3,
        request: TextInputRequest,
        _data: &(),
        _handle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Vwayland>,
    ) {
        match request {
            TextInputRequest::Enable => state.text_input_state.set_pending_enable(resource, true),
            TextInputRequest::Disable => state.text_input_state.set_pending_enable(resource, false),
            TextInputRequest::Commit => state.text_input_state.commit(resource),
            // Surrounding text / content type / cursor rectangle are client state
            // an IME would consume; accepted and ignored here.
            TextInputRequest::SetSurroundingText { .. }
            | TextInputRequest::SetTextChangeCause { .. }
            | TextInputRequest::SetContentType { .. }
            | TextInputRequest::SetCursorRectangle { .. }
            | TextInputRequest::Destroy => {}
            _ => {}
        }
    }

    fn destroyed(
        state: &mut Vwayland,
        _client_id: ClientId,
        resource: &ZwpTextInputV3,
        _data: &(),
    ) {
        state.text_input_state.remove_instance(&resource.id());
    }
}