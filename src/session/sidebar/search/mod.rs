mod item_row;
mod row;

use self::item_row::ItemRow;
use self::row::Row;

use glib::clone;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib, CompositeTemplate};
use tdlib::{enums, functions};

use crate::session::Sidebar;
use crate::tdlib::{Chat, User};
use crate::utils::spawn;
use crate::Session;

mod imp {
    use super::*;
    use glib::subclass::Signal;
    use once_cell::sync::Lazy;
    use std::cell::{Cell, RefCell};

    #[derive(Debug, Default, CompositeTemplate)]
    #[template(resource = "/com/github/melix99/telegrand/ui/sidebar-search.ui")]
    pub(crate) struct Search {
        pub(super) session: RefCell<Option<Session>>,
        pub(super) compact: Cell<bool>,
        #[template_child]
        pub(super) content: TemplateChild<gtk::Box>,
        #[template_child]
        pub(super) search_entry: TemplateChild<gtk::SearchEntry>,
        #[template_child]
        pub(super) stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub(super) selection: TemplateChild<gtk::NoSelection>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Search {
        const NAME: &'static str = "SidebarSearch";
        type Type = super::Search;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            Row::static_type();

            Self::bind_template(klass);
            Self::Type::bind_template_callbacks(klass);

            klass.set_layout_manager_type::<gtk::BinLayout>();
            klass.install_action("sidebar-search.go-back", None, move |widget, _, _| {
                widget.emit_by_name::<()>("close", &[]);
            });
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for Search {
        fn signals() -> &'static [Signal] {
            static SIGNALS: Lazy<Vec<Signal>> = Lazy::new(|| {
                vec![Signal::builder("close", &[], <()>::static_type().into()).build()]
            });
            SIGNALS.as_ref()
        }

        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![
                    glib::ParamSpecObject::new(
                        "session",
                        "Session",
                        "The session",
                        Session::static_type(),
                        glib::ParamFlags::READWRITE | glib::ParamFlags::EXPLICIT_NOTIFY,
                    ),
                    glib::ParamSpecBoolean::new(
                        "compact",
                        "Compact",
                        "Whether a compact view is used or not",
                        false,
                        glib::ParamFlags::READWRITE | glib::ParamFlags::EXPLICIT_NOTIFY,
                    ),
                ]
            });
            PROPERTIES.as_ref()
        }

        fn set_property(
            &self,
            obj: &Self::Type,
            _id: usize,
            value: &glib::Value,
            pspec: &glib::ParamSpec,
        ) {
            match pspec.name() {
                "session" => obj.set_session(value.get().unwrap()),
                "compact" => obj.set_compact(value.get().unwrap()),
                _ => unimplemented!(),
            }
        }

        fn property(&self, obj: &Self::Type, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "session" => obj.session().to_value(),
                "compact" => obj.compact().to_value(),
                _ => unimplemented!(),
            }
        }

        fn dispose(&self, _obj: &Self::Type) {
            self.content.unparent();
        }
    }

    impl WidgetImpl for Search {
        fn grab_focus(&self, _widget: &Self::Type) -> bool {
            self.search_entry.grab_focus();
            true
        }
    }
}

glib::wrapper! {
    pub(crate) struct Search(ObjectSubclass<imp::Search>)
        @extends gtk::Widget;
}

#[gtk::template_callbacks]
impl Search {
    pub(crate) fn reset(&self) {
        let imp = self.imp();
        if imp.search_entry.text().is_empty() {
            // Update recently found chats
            spawn(clone!(@weak self as obj => async move {
                obj.search().await;
            }));
        } else {
            // Reset the search entry. This will also start the search
            // for getting the recently found chats.
            imp.search_entry.set_text("");
        }
    }

    pub(crate) fn session(&self) -> Option<Session> {
        self.imp().session.borrow().clone()
    }

    pub(crate) fn set_session(&self, session: Option<Session>) {
        if self.session() == session {
            return;
        }
        self.imp().session.replace(session);
        self.notify("session");
    }

    pub(crate) fn compact(&self) -> bool {
        self.imp().compact.get()
    }

    pub(crate) fn set_compact(&self, compact: bool) {
        if self.compact() == compact {
            return;
        }
        self.imp().compact.set(compact);
        self.notify("compact");
    }

    #[template_callback]
    async fn search(&self) {
        let imp = self.imp();
        let session = self.session().unwrap();
        let query = imp.search_entry.text().to_string();
        let list = gio::ListStore::new(glib::Object::static_type());
        let mut found_chat_ids: Vec<i64> = vec![];

        imp.selection.set_model(Some(&list));
        list.connect_items_changed(clone!(@weak self as obj => move |list, _, _, _| {
            obj.imp().stack.set_visible_child_name(if list.n_items() > 0 {
                "results"
            } else {
                "empty"
            });
        }));

        // Show the results page prematurely, so that we don't show the empty page
        // before even starting the search.
        imp.stack.set_visible_child_name("results");

        // Search chats locally (or get the recently found chats if the query is empty)
        match functions::search_chats(query.clone(), 30, session.client_id()).await {
            Ok(enums::Chats::Chats(mut data)) if !data.chat_ids.is_empty() => {
                let chats: Vec<Chat> = data
                    .chat_ids
                    .iter()
                    .map(|id| session.chat_list().get(*id))
                    .collect();

                found_chat_ids.append(&mut data.chat_ids);
                list.extend_from_slice(&chats);
            }
            Err(e) => {
                log::warn!("Error searching chats: {:?}", e);
            }
            _ => {}
        }

        // Show the empty page if there are no results after the first part of the search
        if list.n_items() == 0 {
            imp.stack.set_visible_child_name("empty");
        }

        // If the query is empty, we can stop the search here as we just need the
        // recently found chats that the previous search call returned.
        if query.is_empty() {
            return;
        }

        // Search contacts
        match functions::search_contacts(
            query,
            50 - found_chat_ids.len() as i32,
            session.client_id(),
        )
        .await
        {
            Ok(enums::Users::Users(data)) if !data.user_ids.is_empty() => {
                let users: Vec<User> = data
                    .user_ids
                    .into_iter()
                    .filter_map(|id| {
                        // The user IDs are the same as their respective private chat IDs,
                        // so we can just check for chat IDs here.
                        if found_chat_ids.contains(&id) {
                            None
                        } else {
                            found_chat_ids.push(id);
                            Some(session.user_list().get(id))
                        }
                    })
                    .collect();

                list.extend_from_slice(&users);
            }
            Err(e) => {
                log::warn!("Error searching contacts: {:?}", e);
            }
            _ => {}
        }
    }

    #[template_callback]
    async fn list_activate(&self, position: u32) {
        let item = self.imp().selection.item(position).unwrap();
        let session = self.session().unwrap();
        let sidebar = self
            .ancestor(Sidebar::static_type())
            .unwrap()
            .downcast::<Sidebar>()
            .unwrap();

        if let Some(chat) = item.downcast_ref::<Chat>() {
            sidebar.select_chat(chat.clone());

            if let Err(e) = functions::add_recently_found_chat(chat.id(), session.client_id()).await
            {
                log::warn!("Failed to add recently found chat: {:?}", e);
            }
        } else if let Some(user) = item.downcast_ref::<User>() {
            // Check if a private chat with this user already exists
            if let Some(chat) = session.chat_list().try_get(user.id()) {
                sidebar.select_chat(chat);
            } else {
                match functions::create_private_chat(user.id(), true, session.client_id()).await {
                    Ok(enums::Chat::Chat(data)) => {
                        let chat = session.chat_list().get(data.id);
                        sidebar.select_chat(chat);
                    }
                    Err(e) => {
                        log::warn!("Failed to create private chat: {:?}", e);
                    }
                }
            }

            if let Err(e) = functions::add_recently_found_chat(user.id(), session.client_id()).await
            {
                log::warn!("Failed to add recently found chat: {:?}", e);
            }
        } else {
            log::warn!("Unexpected item type: {:?}", item);
        }

        self.emit_by_name::<()>("close", &[]);
    }
}
