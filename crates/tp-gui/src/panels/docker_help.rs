//! DockerHelp panel: local/SSH Docker visibility and health diagnostics.
//!
//! The panel intentionally uses the Docker CLI instead of a daemon socket
//! binding. This keeps local and SSH targets on the same execution path and
//! avoids adding long-lived privileged connections to the GUI process.

use gtk4::prelude::*;
use serde_json::Value;
use std::cell::Cell;
use std::collections::HashMap;
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pax_core::workspace::SshConfig;

use super::PanelBackend;

const PANEL_TYPE_ID: &str = "docker_help";
const COMMAND_TIMEOUT: Duration = Duration::from_secs(25);

#[derive(Debug, Clone, Default)]
pub struct DockerHelpConfig {
    pub context: Option<String>,
    pub ssh: Option<SshConfig>,
    pub refresh_interval: Option<u64>,
}

impl DockerHelpConfig {
    pub fn from_extra(extra: &HashMap<String, String>) -> Self {
        let context = extra
            .get("docker_context")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let refresh_interval = extra
            .get("refresh_interval")
            .and_then(|s| s.trim().parse::<u64>().ok())
            .filter(|seconds| *seconds > 0);
        let ssh = extra
            .get("ssh_host")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .map(|host| SshConfig {
                host,
                port: extra
                    .get("ssh_port")
                    .and_then(|s| s.parse::<u16>().ok())
                    .unwrap_or(22),
                user: extra
                    .get("ssh_user")
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty()),
                password: extra
                    .get("ssh_password")
                    .map(|s| s.to_string())
                    .filter(|s| !s.is_empty()),
                identity_file: extra
                    .get("ssh_identity")
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty()),
                tmux_session: None,
            });

        Self {
            context,
            ssh,
            refresh_interval,
        }
    }
}

pub struct DockerHelpPanel {
    widget: gtk4::Widget,
    inner: Rc<DockerHelpInner>,
}

impl std::fmt::Debug for DockerHelpPanel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DockerHelpPanel")
            .field("target", &self.inner.target)
            .finish()
    }
}

impl DockerHelpPanel {
    pub fn new(config: DockerHelpConfig) -> Self {
        let target = DockerTarget::from_config(config.clone());

        let root = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        root.add_css_class("docker-help-panel");

        let toolbar = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        toolbar.add_css_class("docker-help-toolbar");
        toolbar.set_margin_start(6);
        toolbar.set_margin_end(6);
        toolbar.set_margin_top(4);
        toolbar.set_margin_bottom(4);

        let refresh_btn = gtk4::Button::from_icon_name("view-refresh-symbolic");
        refresh_btn.add_css_class("flat");
        refresh_btn.set_tooltip_text(Some("Refresh Docker state"));
        toolbar.append(&refresh_btn);

        let target_label = gtk4::Label::new(Some(&target.label()));
        target_label.add_css_class("dim-label");
        target_label.add_css_class("caption");
        target_label.set_halign(gtk4::Align::Start);
        target_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        target_label.set_hexpand(true);
        toolbar.append(&target_label);

        let containers_badge = summary_badge("Containers", "-");
        let services_badge = summary_badge("Services", "-");
        let nodes_badge = summary_badge("Nodes", "-");
        let issues_badge = summary_badge("Issues", "-");
        toolbar.append(&containers_badge.container);
        toolbar.append(&services_badge.container);
        toolbar.append(&nodes_badge.container);
        toolbar.append(&issues_badge.container);

        root.append(&toolbar);

        let paned = gtk4::Paned::new(gtk4::Orientation::Vertical);
        paned.set_wide_handle(true);
        paned.set_vexpand(true);
        paned.set_hexpand(true);

        let notebook = gtk4::Notebook::new();
        notebook.set_vexpand(true);
        notebook.set_hexpand(true);

        let overview_box = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
        overview_box.set_margin_start(8);
        overview_box.set_margin_end(8);
        overview_box.set_margin_top(8);
        overview_box.set_margin_bottom(8);
        let overview_status = gtk4::Label::new(Some("Loading Docker state..."));
        overview_status.set_halign(gtk4::Align::Start);
        overview_status.set_wrap(true);
        overview_box.append(&overview_status);
        let diagnostics_list = gtk4::ListBox::new();
        diagnostics_list.add_css_class("docker-list");
        diagnostics_list.set_selection_mode(gtk4::SelectionMode::None);
        let diagnostics_scroll = scrolled_for(&diagnostics_list);
        overview_box.append(&diagnostics_scroll);
        notebook.append_page(&overview_box, Some(&gtk4::Label::new(Some("Health"))));

        let containers_list = gtk4::ListBox::new();
        containers_list.add_css_class("docker-list");
        containers_list.set_selection_mode(gtk4::SelectionMode::None);
        notebook.append_page(
            &scrolled_for(&containers_list),
            Some(&gtk4::Label::new(Some("Containers"))),
        );

        let services_list = gtk4::ListBox::new();
        services_list.add_css_class("docker-list");
        services_list.set_selection_mode(gtk4::SelectionMode::None);
        notebook.append_page(
            &scrolled_for(&services_list),
            Some(&gtk4::Label::new(Some("Services"))),
        );

        let nodes_list = gtk4::ListBox::new();
        nodes_list.add_css_class("docker-list");
        nodes_list.set_selection_mode(gtk4::SelectionMode::None);
        notebook.append_page(
            &scrolled_for(&nodes_list),
            Some(&gtk4::Label::new(Some("Nodes"))),
        );

        let stack_compose_box = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
        stack_compose_box.set_margin_start(6);
        stack_compose_box.set_margin_end(6);
        stack_compose_box.set_margin_top(6);
        stack_compose_box.set_margin_bottom(6);
        let stacks_label = gtk4::Label::new(Some("Swarm stacks"));
        stacks_label.add_css_class("heading");
        stacks_label.set_halign(gtk4::Align::Start);
        stack_compose_box.append(&stacks_label);
        let stacks_list = gtk4::ListBox::new();
        stacks_list.add_css_class("docker-list");
        stacks_list.set_selection_mode(gtk4::SelectionMode::None);
        let stacks_scroll = scrolled_for(&stacks_list);
        stacks_scroll.set_min_content_height(120);
        stack_compose_box.append(&stacks_scroll);
        let compose_label = gtk4::Label::new(Some("Compose projects"));
        compose_label.add_css_class("heading");
        compose_label.set_halign(gtk4::Align::Start);
        stack_compose_box.append(&compose_label);
        let compose_list = gtk4::ListBox::new();
        compose_list.add_css_class("docker-list");
        compose_list.set_selection_mode(gtk4::SelectionMode::None);
        let compose_scroll = scrolled_for(&compose_list);
        compose_scroll.set_min_content_height(120);
        stack_compose_box.append(&compose_scroll);
        notebook.append_page(
            &stack_compose_box,
            Some(&gtk4::Label::new(Some("Stacks/Compose"))),
        );

        let images_list = gtk4::ListBox::new();
        images_list.add_css_class("docker-list");
        images_list.set_selection_mode(gtk4::SelectionMode::None);
        notebook.append_page(
            &scrolled_for(&images_list),
            Some(&gtk4::Label::new(Some("Images"))),
        );

        let details_view = gtk4::TextView::new();
        details_view.set_editable(false);
        details_view.set_cursor_visible(false);
        details_view.set_monospace(true);
        details_view.set_wrap_mode(gtk4::WrapMode::WordChar);
        details_view.add_css_class("docker-details");
        details_view
            .buffer()
            .set_text("Select Logs, Inspect, Env, Tasks, or an action result to inspect details.");
        let details_scroll = gtk4::ScrolledWindow::new();
        details_scroll.set_child(Some(&details_view));
        details_scroll.set_min_content_height(160);
        details_scroll.set_vexpand(true);
        details_scroll.set_hexpand(true);

        paned.set_start_child(Some(&notebook));
        paned.set_end_child(Some(&details_scroll));
        paned.set_position(520);
        root.append(&paned);

        let widget = root.clone().upcast::<gtk4::Widget>();
        let inner = Rc::new(DockerHelpInner {
            root,
            target,
            refresh_interval: config.refresh_interval,
            target_label,
            overview_status,
            diagnostics_list,
            containers_list,
            services_list,
            nodes_list,
            stacks_list,
            compose_list,
            images_list,
            details_view,
            refresh_button: refresh_btn,
            containers_badge,
            services_badge,
            nodes_badge,
            issues_badge,
            refreshing: Cell::new(false),
        });

        {
            let inner_c = inner.clone();
            inner
                .refresh_button
                .connect_clicked(move |_| inner_c.refresh());
        }

        {
            let inner_c = inner.clone();
            gtk4::glib::idle_add_local_once(move || inner_c.refresh());
        }

        if let Some(seconds) = config.refresh_interval.filter(|seconds| *seconds > 0) {
            let weak = Rc::downgrade(&inner);
            gtk4::glib::timeout_add_local(Duration::from_secs(seconds), move || {
                let Some(inner) = weak.upgrade() else {
                    return gtk4::glib::ControlFlow::Break;
                };
                inner.refresh();
                gtk4::glib::ControlFlow::Continue
            });
        }

        Self { widget, inner }
    }
}

impl PanelBackend for DockerHelpPanel {
    fn panel_type(&self) -> &str {
        PANEL_TYPE_ID
    }

    fn widget(&self) -> &gtk4::Widget {
        &self.widget
    }

    fn on_focus(&self) {}

    fn ssh_label(&self) -> Option<String> {
        self.inner.target.ssh_label()
    }

    fn footer_text(&self) -> Option<String> {
        Some(self.inner.target.label())
    }
}

#[derive(Clone)]
struct SummaryBadge {
    container: gtk4::Box,
    value: gtk4::Label,
}

fn summary_badge(title: &str, value: &str) -> SummaryBadge {
    let container = gtk4::Box::new(gtk4::Orientation::Horizontal, 3);
    container.add_css_class("docker-summary-badge");
    let title_label = gtk4::Label::new(Some(title));
    title_label.add_css_class("caption");
    title_label.add_css_class("dim-label");
    let value_label = gtk4::Label::new(Some(value));
    value_label.add_css_class("caption");
    value_label.add_css_class("docker-summary-value");
    container.append(&title_label);
    container.append(&value_label);
    SummaryBadge {
        container,
        value: value_label,
    }
}

struct DockerHelpInner {
    root: gtk4::Box,
    target: DockerTarget,
    refresh_interval: Option<u64>,
    target_label: gtk4::Label,
    overview_status: gtk4::Label,
    diagnostics_list: gtk4::ListBox,
    containers_list: gtk4::ListBox,
    services_list: gtk4::ListBox,
    nodes_list: gtk4::ListBox,
    stacks_list: gtk4::ListBox,
    compose_list: gtk4::ListBox,
    images_list: gtk4::ListBox,
    details_view: gtk4::TextView,
    refresh_button: gtk4::Button,
    containers_badge: SummaryBadge,
    services_badge: SummaryBadge,
    nodes_badge: SummaryBadge,
    issues_badge: SummaryBadge,
    refreshing: Cell<bool>,
}

impl DockerHelpInner {
    fn refresh(self: &Rc<Self>) {
        if self.refreshing.replace(true) {
            return;
        }
        self.refresh_button.set_sensitive(false);
        self.overview_status.set_text("Refreshing Docker state...");
        self.target_label.set_text(&self.target.label());

        let target = self.target.clone();
        let slot = Arc::new(Mutex::new(None::<DockerSnapshot>));
        let slot_for_thread = slot.clone();
        std::thread::spawn(move || {
            let snapshot = target.snapshot();
            *slot_for_thread.lock().unwrap() = Some(snapshot);
        });

        let weak = Rc::downgrade(self);
        gtk4::glib::timeout_add_local(Duration::from_millis(120), move || {
            let maybe_snapshot = slot.lock().unwrap().take();
            let Some(snapshot) = maybe_snapshot else {
                return if weak.upgrade().is_some() {
                    gtk4::glib::ControlFlow::Continue
                } else {
                    gtk4::glib::ControlFlow::Break
                };
            };
            if let Some(inner) = weak.upgrade() {
                inner.apply_snapshot(snapshot);
                inner.refresh_button.set_sensitive(true);
                inner.refreshing.set(false);
            }
            gtk4::glib::ControlFlow::Break
        });
    }

    fn apply_snapshot(self: &Rc<Self>, snapshot: DockerSnapshot) {
        let diagnostics = build_diagnostics(&snapshot);
        let issue_count = diagnostics
            .iter()
            .filter(|d| d.level != HealthLevel::Ok)
            .count();
        let degraded_services = snapshot
            .services
            .iter()
            .filter(|s| service_health(&s.replicas) != HealthLevel::Ok)
            .count();
        let unhealthy_containers = snapshot
            .containers
            .iter()
            .filter(|c| container_health(&c.status, &c.state) == HealthLevel::Critical)
            .count();
        let not_ready_nodes = snapshot
            .nodes
            .iter()
            .filter(|n| node_health(n) != HealthLevel::Ok)
            .count();

        self.containers_badge.value.set_text(&format!(
            "{} / {} bad",
            snapshot.containers.len(),
            unhealthy_containers
        ));
        self.services_badge.value.set_text(&format!(
            "{} / {} bad",
            snapshot.services.len(),
            degraded_services
        ));
        self.nodes_badge.value.set_text(&format!(
            "{} / {} bad",
            snapshot.nodes.len(),
            not_ready_nodes
        ));
        self.issues_badge.value.set_text(&issue_count.to_string());

        self.overview_status
            .set_text(&format_overview(&snapshot, self.refresh_interval));
        populate_diagnostics(&self.diagnostics_list, &diagnostics);
        self.populate_containers(&snapshot.containers);
        self.populate_services(&snapshot.services);
        self.populate_nodes(&snapshot.nodes);
        self.populate_stacks(&snapshot.stacks);
        self.populate_compose(&snapshot.compose_projects);
        self.populate_images(&snapshot.images);
    }

    fn populate_containers(self: &Rc<Self>, containers: &[DockerContainer]) {
        clear_list(&self.containers_list);
        if containers.is_empty() {
            append_empty(&self.containers_list, "No containers found.");
            return;
        }

        for container in containers {
            let row = list_row();
            let body = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
            let health = container_health(&container.status, &container.state);
            body.append(&health_icon(health));

            let labels = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
            labels.set_hexpand(true);
            labels.append(&primary_label(&format!(
                "{}  {}",
                display_or(&container.names, "<unnamed>"),
                short_id(&container.id)
            )));
            labels.append(&secondary_label(&format!(
                "{} | {} | {}",
                display_or(&container.image, "<no image>"),
                display_or(&container.state, "-"),
                display_or(&container.status, "-")
            )));
            if !container.ports.is_empty() {
                labels.append(&secondary_label(&container.ports));
            }
            body.append(&labels);

            let actions = gtk4::Box::new(gtk4::Orientation::Horizontal, 3);
            let id = container.id.clone();
            let name = display_or(&container.names, &id).to_string();
            actions.append(&self.command_button(
                "view-list-symbolic",
                "Logs",
                format!("Logs: {name}"),
                vec!["logs", "--tail", "300", &id],
                false,
            ));
            actions.append(&self.command_button(
                "dialog-information-symbolic",
                "Inspect",
                format!("Inspect: {name}"),
                vec!["inspect", &id],
                false,
            ));
            actions.append(&self.command_button(
                "text-x-generic-symbolic",
                "Env",
                format!("Environment: {name}"),
                vec![
                    "inspect",
                    "--format",
                    "{{range .Config.Env}}{{println .}}{{end}}",
                    &id,
                ],
                false,
            ));
            actions.append(&self.command_button(
                "view-refresh-symbolic",
                "Restart",
                format!("Restart: {name}"),
                vec!["restart", &id],
                true,
            ));
            actions.append(&self.command_button(
                "media-playback-stop-symbolic",
                "Stop",
                format!("Stop: {name}"),
                vec!["stop", &id],
                true,
            ));
            actions.append(&self.confirm_button(
                "user-trash-symbolic",
                "Delete",
                format!("Delete container {name}?"),
                "This runs docker rm -f and cannot be undone from Pax.",
                format!("Delete: {name}"),
                vec!["rm", "-f", &id],
            ));
            body.append(&actions);
            row.set_child(Some(&body));
            self.containers_list.append(&row);
        }
    }

    fn populate_services(self: &Rc<Self>, services: &[DockerService]) {
        clear_list(&self.services_list);
        if services.is_empty() {
            append_empty(
                &self.services_list,
                "No swarm services found. If this is a compose-only host, use Containers/Stacks.",
            );
            return;
        }

        for service in services {
            let row = list_row();
            let body = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
            let health = service_health(&service.replicas);
            body.append(&health_icon(health));

            let labels = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
            labels.set_hexpand(true);
            labels.append(&primary_label(&service.name));
            labels.append(&secondary_label(&format!(
                "{} | replicas {} | {}",
                display_or(&service.mode, "-"),
                display_or(&service.replicas, "-"),
                display_or(&service.image, "<no image>")
            )));
            if !service.ports.is_empty() {
                labels.append(&secondary_label(&service.ports));
            }
            body.append(&labels);

            let actions = gtk4::Box::new(gtk4::Orientation::Horizontal, 3);
            let name = service.name.clone();
            actions.append(&self.command_button(
                "view-list-symbolic",
                "Logs",
                format!("Service logs: {name}"),
                vec!["service", "logs", "--tail", "300", &name],
                false,
            ));
            actions.append(&self.command_button(
                "format-justify-fill-symbolic",
                "Tasks",
                format!("Service tasks: {name}"),
                vec!["service", "ps", "--no-trunc", &name],
                false,
            ));
            actions.append(&self.command_button(
                "dialog-information-symbolic",
                "Inspect",
                format!("Inspect service: {name}"),
                vec!["service", "inspect", &name],
                false,
            ));
            actions.append(&self.confirm_button(
                "view-refresh-symbolic",
                "Reload service",
                format!("Force update service {name}?"),
                "This restarts service tasks with docker service update --force.",
                format!("Reload service: {name}"),
                vec!["service", "update", "--force", &name],
            ));
            body.append(&actions);
            row.set_child(Some(&body));
            self.services_list.append(&row);
        }
    }

    fn populate_nodes(self: &Rc<Self>, nodes: &[DockerNode]) {
        clear_list(&self.nodes_list);
        if nodes.is_empty() {
            append_empty(&self.nodes_list, "No swarm nodes found.");
            return;
        }

        for node in nodes {
            let row = list_row();
            let body = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
            body.append(&health_icon(node_health(node)));

            let labels = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
            labels.set_hexpand(true);
            labels.append(&primary_label(&format!(
                "{}  {}",
                display_or(&node.hostname, "<unknown>"),
                short_id(&node.id)
            )));
            labels.append(&secondary_label(&format!(
                "status {} | availability {} | manager {} | engine {}",
                display_or(&node.status, "-"),
                display_or(&node.availability, "-"),
                display_or(&node.manager_status, "-"),
                display_or(&node.engine_version, "-")
            )));
            body.append(&labels);
            row.set_child(Some(&body));
            self.nodes_list.append(&row);
        }
    }

    fn populate_stacks(self: &Rc<Self>, stacks: &[DockerStack]) {
        clear_list(&self.stacks_list);
        if stacks.is_empty() {
            append_empty(&self.stacks_list, "No swarm stacks found.");
            return;
        }

        for stack in stacks {
            let row = list_row();
            let body = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
            body.append(&gtk4::Image::from_icon_name("applications-system-symbolic"));

            let labels = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
            labels.set_hexpand(true);
            labels.append(&primary_label(&stack.name));
            labels.append(&secondary_label(&format!(
                "{} services | {}",
                display_or(&stack.services, "-"),
                display_or(&stack.orchestrator, "swarm")
            )));
            body.append(&labels);

            let actions = gtk4::Box::new(gtk4::Orientation::Horizontal, 3);
            let name = stack.name.clone();
            actions.append(&self.command_button(
                "format-justify-fill-symbolic",
                "Services",
                format!("Stack services: {name}"),
                vec!["stack", "services", &name],
                false,
            ));
            actions.append(&self.command_button(
                "view-list-symbolic",
                "Tasks",
                format!("Stack tasks: {name}"),
                vec!["stack", "ps", "--no-trunc", &name],
                false,
            ));
            actions.append(&self.confirm_button(
                "user-trash-symbolic",
                "Remove stack",
                format!("Remove stack {name}?"),
                "This runs docker stack rm and removes the stack services.",
                format!("Remove stack: {name}"),
                vec!["stack", "rm", &name],
            ));
            body.append(&actions);
            row.set_child(Some(&body));
            self.stacks_list.append(&row);
        }
    }

    fn populate_compose(self: &Rc<Self>, projects: &[ComposeProject]) {
        clear_list(&self.compose_list);
        if projects.is_empty() {
            append_empty(
                &self.compose_list,
                "No compose projects reported by docker compose ls.",
            );
            return;
        }

        for project in projects {
            let row = list_row();
            let body = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
            let health = if project.status.to_ascii_lowercase().contains("running") {
                HealthLevel::Ok
            } else {
                HealthLevel::Warn
            };
            body.append(&health_icon(health));

            let labels = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
            labels.set_hexpand(true);
            labels.append(&primary_label(&project.name));
            labels.append(&secondary_label(&format!(
                "{} | {}",
                display_or(&project.status, "-"),
                display_or(&project.config_files, "-")
            )));
            body.append(&labels);

            let actions = gtk4::Box::new(gtk4::Orientation::Horizontal, 3);
            let name = project.name.clone();
            actions.append(&self.command_button(
                "view-list-symbolic",
                "Containers",
                format!("Compose containers: {name}"),
                vec![
                    "ps",
                    "-a",
                    "--filter",
                    &format!("label=com.docker.compose.project={name}"),
                ],
                false,
            ));
            body.append(&actions);
            row.set_child(Some(&body));
            self.compose_list.append(&row);
        }
    }

    fn populate_images(self: &Rc<Self>, images: &[DockerImage]) {
        clear_list(&self.images_list);
        if images.is_empty() {
            append_empty(&self.images_list, "No images found.");
            return;
        }

        for image in images {
            let row = list_row();
            let body = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
            body.append(&gtk4::Image::from_icon_name("package-x-generic-symbolic"));

            let labels = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
            labels.set_hexpand(true);
            labels.append(&primary_label(&format!(
                "{}:{}",
                display_or(&image.repository, "<none>"),
                display_or(&image.tag, "<none>")
            )));
            labels.append(&secondary_label(&format!(
                "{} | {} | {}",
                short_id(&image.id),
                display_or(&image.size, "-"),
                display_or(&image.created_since, "-")
            )));
            body.append(&labels);

            let actions = gtk4::Box::new(gtk4::Orientation::Horizontal, 3);
            let id = image.id.clone();
            let display = format!("{}:{}", image.repository, image.tag);
            actions.append(&self.command_button(
                "dialog-information-symbolic",
                "Inspect",
                format!("Inspect image: {display}"),
                vec!["image", "inspect", &id],
                false,
            ));
            actions.append(&self.confirm_button(
                "user-trash-symbolic",
                "Remove image",
                format!("Remove image {display}?"),
                "This runs docker image rm and may fail if containers still use the image.",
                format!("Remove image: {display}"),
                vec!["image", "rm", &id],
            ));
            body.append(&actions);
            row.set_child(Some(&body));
            self.images_list.append(&row);
        }
    }

    fn command_button(
        self: &Rc<Self>,
        icon: &str,
        tooltip: &str,
        title: String,
        args: Vec<&str>,
        refresh_after: bool,
    ) -> gtk4::Button {
        let button = icon_button(icon, tooltip);
        let inner = self.clone();
        let args = args.into_iter().map(str::to_string).collect::<Vec<_>>();
        button.connect_clicked(move |_| {
            inner.run_detail(title.clone(), args.clone(), refresh_after);
        });
        button
    }

    fn confirm_button(
        self: &Rc<Self>,
        icon: &str,
        tooltip: &str,
        title: String,
        message: &str,
        result_title: String,
        args: Vec<&str>,
    ) -> gtk4::Button {
        let button = icon_button(icon, tooltip);
        let inner = self.clone();
        let args = args.into_iter().map(str::to_string).collect::<Vec<_>>();
        let message = message.to_string();
        button.connect_clicked(move |_| {
            inner.confirm_and_run(
                title.clone(),
                message.clone(),
                result_title.clone(),
                args.clone(),
            );
        });
        button
    }

    fn run_detail(self: &Rc<Self>, title: String, args: Vec<String>, refresh_after: bool) {
        let command_line = self.target.display_command(&args);
        self.set_details(&format!("{title}\n\n$ {command_line}\n\nRunning..."));

        let target = self.target.clone();
        let slot = Arc::new(Mutex::new(None::<DockerCommandResult>));
        let slot_for_thread = slot.clone();
        std::thread::spawn(move || {
            let result = target.run_docker(&args);
            *slot_for_thread.lock().unwrap() = Some(result);
        });

        let weak = Rc::downgrade(self);
        gtk4::glib::timeout_add_local(Duration::from_millis(120), move || {
            let maybe_result = slot.lock().unwrap().take();
            let Some(result) = maybe_result else {
                return if weak.upgrade().is_some() {
                    gtk4::glib::ControlFlow::Continue
                } else {
                    gtk4::glib::ControlFlow::Break
                };
            };
            if let Some(inner) = weak.upgrade() {
                let status = if result.success() { "OK" } else { "FAILED" };
                inner.set_details(&format!(
                    "{title}\n\n$ {command_line}\n\n[{status}]\n{}",
                    result.combined_output()
                ));
                if refresh_after {
                    inner.refresh();
                }
            }
            gtk4::glib::ControlFlow::Break
        });
    }

    fn confirm_and_run(
        self: &Rc<Self>,
        title: String,
        message: String,
        result_title: String,
        args: Vec<String>,
    ) {
        let dialog = gtk4::Window::builder()
            .title("Confirm Docker Action")
            .modal(true)
            .default_width(460)
            .default_height(140)
            .build();
        crate::theme::configure_dialog_window(&dialog);
        if let Some(parent) = self.parent_window() {
            dialog.set_transient_for(Some(&parent));
        }

        let root = gtk4::Box::new(gtk4::Orientation::Vertical, 10);
        root.set_margin_top(14);
        root.set_margin_bottom(14);
        root.set_margin_start(14);
        root.set_margin_end(14);
        let title_label = primary_label(&title);
        title_label.set_wrap(true);
        root.append(&title_label);
        let message_label = secondary_label(&message);
        message_label.set_wrap(true);
        root.append(&message_label);

        let buttons = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        buttons.set_halign(gtk4::Align::End);
        let cancel = gtk4::Button::with_label("Cancel");
        cancel.add_css_class("flat");
        let run = gtk4::Button::with_label("Run");
        run.add_css_class("destructive-action");
        buttons.append(&cancel);
        buttons.append(&run);
        root.append(&buttons);

        {
            let dialog_c = dialog.clone();
            cancel.connect_clicked(move |_| dialog_c.close());
        }
        {
            let dialog_c = dialog.clone();
            let inner = self.clone();
            run.connect_clicked(move |_| {
                dialog_c.close();
                inner.run_detail(result_title.clone(), args.clone(), true);
            });
        }

        dialog.set_child(Some(&root));
        dialog.present();
    }

    fn parent_window(&self) -> Option<gtk4::Window> {
        self.root
            .root()
            .and_then(|root| root.downcast::<gtk4::Window>().ok())
    }

    fn set_details(&self, text: &str) {
        self.details_view.buffer().set_text(text);
    }
}

#[derive(Debug, Clone)]
enum DockerTarget {
    Local {
        context: Option<String>,
    },
    Ssh {
        context: Option<String>,
        ssh: SshConfig,
    },
}

impl DockerTarget {
    fn from_config(config: DockerHelpConfig) -> Self {
        match config.ssh {
            Some(ssh) => Self::Ssh {
                context: config.context,
                ssh,
            },
            None => Self::Local {
                context: config.context,
            },
        }
    }

    fn label(&self) -> String {
        match self {
            Self::Local { context } => match context {
                Some(context) => format!("Docker local context: {context}"),
                None => "Docker local".to_string(),
            },
            Self::Ssh { context, ssh } => {
                let target = ssh_target_label(ssh);
                match context {
                    Some(context) => format!("Docker SSH: {target} | context: {context}"),
                    None => format!("Docker SSH: {target}"),
                }
            }
        }
    }

    fn ssh_label(&self) -> Option<String> {
        match self {
            Self::Local { .. } => None,
            Self::Ssh { ssh, .. } => Some(ssh_target_label(ssh)),
        }
    }

    fn snapshot(&self) -> DockerSnapshot {
        let mut snapshot = DockerSnapshot::default();

        let version = self.run_docker(&["version", "--format", "{{.Server.Version}}"]);
        if version.success() {
            snapshot.server_version = version.stdout.trim().to_string();
        } else {
            snapshot
                .errors
                .push(format!("Docker version failed: {}", version.short_error()));
        }

        let info = self.run_docker(&["info", "--format", "{{json .}}"]);
        if info.success() {
            match serde_json::from_str::<Value>(&info.stdout) {
                Ok(json) => {
                    snapshot.info = DockerInfo::from_json(&json);
                    snapshot.info_raw =
                        serde_json::to_string_pretty(&json).unwrap_or_else(|_| info.stdout.clone());
                }
                Err(e) => {
                    snapshot.info_raw = info.stdout.clone();
                    snapshot
                        .errors
                        .push(format!("Could not parse docker info JSON: {e}"));
                }
            }
        } else {
            snapshot
                .errors
                .push(format!("Docker info failed: {}", info.short_error()));
            return snapshot;
        }

        let containers = self.run_docker(&["ps", "-a", "--no-trunc", "--format", "{{json .}}"]);
        if containers.success() {
            snapshot.containers = parse_json_lines(&containers.stdout)
                .into_iter()
                .map(|v| DockerContainer::from_json(&v))
                .collect();
        } else {
            snapshot
                .errors
                .push(format!("docker ps failed: {}", containers.short_error()));
        }

        let images = self.run_docker(&["image", "ls", "--format", "{{json .}}"]);
        if images.success() {
            snapshot.images = parse_json_lines(&images.stdout)
                .into_iter()
                .map(|v| DockerImage::from_json(&v))
                .collect();
        } else {
            snapshot
                .errors
                .push(format!("docker image ls failed: {}", images.short_error()));
        }

        if snapshot.info.swarm_active() {
            let services = self.run_docker(&["service", "ls", "--format", "{{json .}}"]);
            if services.success() {
                snapshot.services = parse_json_lines(&services.stdout)
                    .into_iter()
                    .map(|v| DockerService::from_json(&v))
                    .collect();
            } else {
                snapshot.errors.push(format!(
                    "docker service ls failed: {}",
                    services.short_error()
                ));
            }

            let stacks = self.run_docker(&["stack", "ls", "--format", "{{json .}}"]);
            if stacks.success() {
                snapshot.stacks = parse_json_lines(&stacks.stdout)
                    .into_iter()
                    .map(|v| DockerStack::from_json(&v))
                    .collect();
            } else {
                snapshot
                    .errors
                    .push(format!("docker stack ls failed: {}", stacks.short_error()));
            }

            let nodes = self.run_docker(&["node", "ls", "--format", "{{json .}}"]);
            if nodes.success() {
                snapshot.nodes = parse_json_lines(&nodes.stdout)
                    .into_iter()
                    .map(|v| DockerNode::from_json(&v))
                    .collect();
            } else {
                snapshot
                    .errors
                    .push(format!("docker node ls failed: {}", nodes.short_error()));
            }
        }

        let compose = self.run_docker(&["compose", "ls", "--format", "json"]);
        if compose.success() {
            snapshot.compose_projects = parse_compose_projects(&compose.stdout);
        }

        snapshot
    }

    fn display_command(&self, args: &[String]) -> String {
        match self {
            Self::Local { context } => docker_command_string(context.as_deref(), args),
            Self::Ssh { context, ssh } => {
                format!(
                    "ssh {} {}",
                    shell_quote(&ssh_target_label(ssh)),
                    shell_quote(&docker_command_string(context.as_deref(), args))
                )
            }
        }
    }

    fn run_docker<S: AsRef<str>>(&self, args: &[S]) -> DockerCommandResult {
        let args = args
            .iter()
            .map(|arg| arg.as_ref().to_string())
            .collect::<Vec<_>>();
        match self {
            Self::Local { context } => {
                let mut cmd = Command::new("docker");
                if let Some(context) = context.as_deref().filter(|s| !s.trim().is_empty()) {
                    cmd.args(["--context", context]);
                }
                cmd.args(&args);
                run_command_with_timeout(cmd, COMMAND_TIMEOUT)
            }
            Self::Ssh { context, ssh } => {
                let remote = docker_command_string(context.as_deref(), &args);
                let mut cmd = if let Some(password) = ssh.password.as_deref() {
                    let mut cmd = Command::new("sshpass");
                    cmd.args(["-p", password, "ssh"]);
                    cmd
                } else {
                    Command::new("ssh")
                };
                cmd.args([
                    "-o",
                    "StrictHostKeyChecking=accept-new",
                    "-o",
                    "ConnectTimeout=8",
                    "-p",
                    &ssh.port.to_string(),
                ]);
                if let Some(identity) = ssh.identity_file.as_deref() {
                    cmd.args(["-i", identity]);
                }
                cmd.arg(ssh_target_label(ssh));
                cmd.arg(remote);
                run_command_with_timeout(cmd, COMMAND_TIMEOUT)
            }
        }
    }
}

fn docker_command_string(context: Option<&str>, args: &[String]) -> String {
    let mut parts = vec!["docker".to_string()];
    if let Some(context) = context.filter(|s| !s.trim().is_empty()) {
        parts.push("--context".to_string());
        parts.push(context.to_string());
    }
    parts.extend(args.iter().cloned());
    parts
        .iter()
        .map(|part| shell_quote(part))
        .collect::<Vec<_>>()
        .join(" ")
}

fn ssh_target_label(ssh: &SshConfig) -> String {
    match ssh.user.as_deref().filter(|s| !s.is_empty()) {
        Some(user) => format!("{user}@{}", ssh.host),
        None => ssh.host.clone(),
    }
}

fn shell_quote(value: &str) -> String {
    if !value.is_empty()
        && value.chars().all(|c| {
            c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '_' | '-' | ':' | '=' | '@')
        })
    {
        return value.to_string();
    }
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[derive(Debug, Clone, Default)]
struct DockerSnapshot {
    server_version: String,
    info: DockerInfo,
    info_raw: String,
    containers: Vec<DockerContainer>,
    images: Vec<DockerImage>,
    services: Vec<DockerService>,
    stacks: Vec<DockerStack>,
    nodes: Vec<DockerNode>,
    compose_projects: Vec<ComposeProject>,
    errors: Vec<String>,
}

#[derive(Debug, Clone, Default)]
struct DockerInfo {
    server_version: String,
    operating_system: String,
    architecture: String,
    containers: u64,
    containers_running: u64,
    images: u64,
    storage_driver: String,
    swarm_state: String,
    swarm_control_available: bool,
    swarm_nodes: u64,
    swarm_managers: u64,
}

impl DockerInfo {
    fn from_json(value: &Value) -> Self {
        let swarm = value.get("Swarm").unwrap_or(&Value::Null);
        Self {
            server_version: field(value, &["ServerVersion"]),
            operating_system: field(value, &["OperatingSystem"]),
            architecture: field(value, &["Architecture"]),
            containers: field_u64(value, &["Containers"]),
            containers_running: field_u64(value, &["ContainersRunning"]),
            images: field_u64(value, &["Images"]),
            storage_driver: field(value, &["Driver"]),
            swarm_state: field(swarm, &["LocalNodeState"]),
            swarm_control_available: field_bool(swarm, &["ControlAvailable"]),
            swarm_nodes: field_u64(swarm, &["Nodes"]),
            swarm_managers: field_u64(swarm, &["Managers"]),
        }
    }

    fn swarm_active(&self) -> bool {
        self.swarm_state.eq_ignore_ascii_case("active")
    }
}

#[derive(Debug, Clone, Default)]
struct DockerContainer {
    id: String,
    names: String,
    image: String,
    status: String,
    state: String,
    ports: String,
}

impl DockerContainer {
    fn from_json(value: &Value) -> Self {
        Self {
            id: field(value, &["ID", "Id"]),
            names: field(value, &["Names", "Name"]),
            image: field(value, &["Image"]),
            status: field(value, &["Status"]),
            state: field(value, &["State"]),
            ports: field(value, &["Ports"]),
        }
    }
}

#[derive(Debug, Clone, Default)]
struct DockerImage {
    id: String,
    repository: String,
    tag: String,
    size: String,
    created_since: String,
}

impl DockerImage {
    fn from_json(value: &Value) -> Self {
        Self {
            id: field(value, &["ID", "Id"]),
            repository: field(value, &["Repository"]),
            tag: field(value, &["Tag"]),
            size: field(value, &["Size"]),
            created_since: field(value, &["CreatedSince", "CreatedAt"]),
        }
    }
}

#[derive(Debug, Clone, Default)]
struct DockerService {
    name: String,
    mode: String,
    replicas: String,
    image: String,
    ports: String,
}

impl DockerService {
    fn from_json(value: &Value) -> Self {
        Self {
            name: field(value, &["Name"]),
            mode: field(value, &["Mode"]),
            replicas: field(value, &["Replicas"]),
            image: field(value, &["Image"]),
            ports: field(value, &["Ports"]),
        }
    }
}

#[derive(Debug, Clone, Default)]
struct DockerStack {
    name: String,
    services: String,
    orchestrator: String,
}

impl DockerStack {
    fn from_json(value: &Value) -> Self {
        Self {
            name: field(value, &["Name"]),
            services: field(value, &["Services"]),
            orchestrator: field(value, &["Orchestrator"]),
        }
    }
}

#[derive(Debug, Clone, Default)]
struct DockerNode {
    id: String,
    hostname: String,
    status: String,
    availability: String,
    manager_status: String,
    engine_version: String,
}

impl DockerNode {
    fn from_json(value: &Value) -> Self {
        Self {
            id: field(value, &["ID", "Id"]),
            hostname: field(value, &["Hostname", "Name"]),
            status: field(value, &["Status"]),
            availability: field(value, &["Availability"]),
            manager_status: field(value, &["ManagerStatus"]),
            engine_version: field(value, &["EngineVersion"]),
        }
    }
}

#[derive(Debug, Clone, Default)]
struct ComposeProject {
    name: String,
    status: String,
    config_files: String,
}

fn parse_compose_projects(stdout: &str) -> Vec<ComposeProject> {
    let Ok(value) = serde_json::from_str::<Value>(stdout) else {
        return Vec::new();
    };
    let Some(items) = value.as_array() else {
        return Vec::new();
    };
    items
        .iter()
        .map(|item| ComposeProject {
            name: field(item, &["Name", "name"]),
            status: field(item, &["Status", "status"]),
            config_files: field(item, &["ConfigFiles", "configFiles", "config_files"]),
        })
        .filter(|item| !item.name.is_empty())
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HealthLevel {
    Ok,
    Warn,
    Critical,
}

fn container_health(status: &str, state: &str) -> HealthLevel {
    let status_l = status.to_ascii_lowercase();
    let state_l = state.to_ascii_lowercase();
    if status_l.contains("unhealthy")
        || state_l.contains("restarting")
        || state_l.contains("dead")
        || state_l.contains("removing")
    {
        return HealthLevel::Critical;
    }
    if state_l.contains("exited") || state_l.contains("paused") || status_l.starts_with("exited") {
        return HealthLevel::Warn;
    }
    HealthLevel::Ok
}

fn service_health(replicas: &str) -> HealthLevel {
    let Some((running, desired)) = parse_replicas(replicas) else {
        return HealthLevel::Ok;
    };
    if running >= desired {
        HealthLevel::Ok
    } else if running == 0 && desired > 0 {
        HealthLevel::Critical
    } else {
        HealthLevel::Warn
    }
}

fn node_health(node: &DockerNode) -> HealthLevel {
    let status = node.status.to_ascii_lowercase();
    let availability = node.availability.to_ascii_lowercase();
    if status != "ready" {
        return HealthLevel::Critical;
    }
    if availability != "active" {
        return HealthLevel::Warn;
    }
    HealthLevel::Ok
}

fn parse_replicas(value: &str) -> Option<(u64, u64)> {
    let (left, right) = value.split_once('/')?;
    Some((left.trim().parse().ok()?, right.trim().parse().ok()?))
}

#[derive(Debug, Clone)]
struct Diagnostic {
    level: HealthLevel,
    title: String,
    detail: String,
}

fn build_diagnostics(snapshot: &DockerSnapshot) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for error in &snapshot.errors {
        diagnostics.push(Diagnostic {
            level: HealthLevel::Critical,
            title: "Docker command failed".to_string(),
            detail: error.clone(),
        });
    }

    for container in &snapshot.containers {
        match container_health(&container.status, &container.state) {
            HealthLevel::Critical => diagnostics.push(Diagnostic {
                level: HealthLevel::Critical,
                title: format!(
                    "Container problem: {}",
                    display_or(&container.names, &container.id)
                ),
                detail: format!(
                    "{} | state={} | status={}",
                    display_or(&container.image, "<no image>"),
                    display_or(&container.state, "-"),
                    display_or(&container.status, "-")
                ),
            }),
            HealthLevel::Warn => diagnostics.push(Diagnostic {
                level: HealthLevel::Warn,
                title: format!(
                    "Container not running: {}",
                    display_or(&container.names, &container.id)
                ),
                detail: format!(
                    "{} | state={} | status={}",
                    display_or(&container.image, "<no image>"),
                    display_or(&container.state, "-"),
                    display_or(&container.status, "-")
                ),
            }),
            HealthLevel::Ok => {}
        }
    }

    for service in &snapshot.services {
        match service_health(&service.replicas) {
            HealthLevel::Critical | HealthLevel::Warn => diagnostics.push(Diagnostic {
                level: service_health(&service.replicas),
                title: format!("Service degraded: {}", service.name),
                detail: format!(
                    "replicas={} | mode={} | image={}",
                    display_or(&service.replicas, "-"),
                    display_or(&service.mode, "-"),
                    display_or(&service.image, "-")
                ),
            }),
            HealthLevel::Ok => {}
        }
    }

    for node in &snapshot.nodes {
        match node_health(node) {
            HealthLevel::Critical | HealthLevel::Warn => diagnostics.push(Diagnostic {
                level: node_health(node),
                title: format!("Swarm node issue: {}", display_or(&node.hostname, &node.id)),
                detail: format!(
                    "status={} | availability={} | manager={}",
                    display_or(&node.status, "-"),
                    display_or(&node.availability, "-"),
                    display_or(&node.manager_status, "-")
                ),
            }),
            HealthLevel::Ok => {}
        }
    }

    if diagnostics.is_empty() {
        diagnostics.push(Diagnostic {
            level: HealthLevel::Ok,
            title: "No critical Docker health issue detected".to_string(),
            detail: "Container health, swarm services, and nodes are currently clean.".to_string(),
        });
    }

    diagnostics
}

fn format_overview(snapshot: &DockerSnapshot, refresh_interval: Option<u64>) -> String {
    let version = display_or(
        &snapshot.server_version,
        display_or(&snapshot.info.server_version, "-"),
    );
    let auto = refresh_interval
        .map(|s| format!("auto refresh: {s}s"))
        .unwrap_or_else(|| "manual refresh".to_string());
    format!(
        "Docker {version} | {} {} | driver {} | containers {}/{} running | images {} | swarm {} (nodes {}, managers {}, control {}) | {auto}",
        display_or(&snapshot.info.operating_system, "-"),
        display_or(&snapshot.info.architecture, "-"),
        display_or(&snapshot.info.storage_driver, "-"),
        snapshot.info.containers_running,
        snapshot.info.containers,
        snapshot.info.images,
        display_or(&snapshot.info.swarm_state, "inactive"),
        snapshot.info.swarm_nodes,
        snapshot.info.swarm_managers,
        snapshot.info.swarm_control_available
    )
}

fn populate_diagnostics(list: &gtk4::ListBox, diagnostics: &[Diagnostic]) {
    clear_list(list);
    for diagnostic in diagnostics {
        let row = list_row();
        let body = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        body.append(&health_icon(diagnostic.level));
        let labels = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
        labels.set_hexpand(true);
        labels.append(&primary_label(&diagnostic.title));
        labels.append(&secondary_label(&diagnostic.detail));
        body.append(&labels);
        row.set_child(Some(&body));
        list.append(&row);
    }
}

fn parse_json_lines(stdout: &str) -> Vec<Value> {
    stdout
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .collect()
}

fn field(value: &Value, keys: &[&str]) -> String {
    keys.iter()
        .find_map(|key| value.get(*key))
        .and_then(value_to_string)
        .unwrap_or_default()
}

fn field_u64(value: &Value, keys: &[&str]) -> u64 {
    keys.iter()
        .find_map(|key| value.get(*key))
        .and_then(|value| value.as_u64().or_else(|| value.as_str()?.parse().ok()))
        .unwrap_or(0)
}

fn field_bool(value: &Value, keys: &[&str]) -> bool {
    keys.iter()
        .find_map(|key| value.get(*key))
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}

fn value_to_string(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        other => Some(other.to_string()),
    }
}

#[derive(Debug, Clone)]
struct DockerCommandResult {
    stdout: String,
    stderr: String,
    status: Option<i32>,
    timed_out: bool,
    spawn_error: Option<String>,
}

impl DockerCommandResult {
    fn success(&self) -> bool {
        self.spawn_error.is_none() && !self.timed_out && self.status == Some(0)
    }

    fn combined_output(&self) -> String {
        let mut out = String::new();
        if let Some(err) = &self.spawn_error {
            out.push_str(err);
            out.push('\n');
        }
        if self.timed_out {
            out.push_str("Command timed out.\n");
        }
        if !self.stdout.trim().is_empty() {
            out.push_str(&self.stdout);
            if !self.stdout.ends_with('\n') {
                out.push('\n');
            }
        }
        if !self.stderr.trim().is_empty() {
            out.push_str("\n[stderr]\n");
            out.push_str(&self.stderr);
        }
        if out.trim().is_empty() {
            "(no output)".to_string()
        } else {
            out
        }
    }

    fn short_error(&self) -> String {
        if let Some(err) = &self.spawn_error {
            return err.clone();
        }
        if self.timed_out {
            return "command timed out".to_string();
        }
        let err = self.stderr.trim();
        if !err.is_empty() {
            return err.chars().take(180).collect();
        }
        format!("exit status {:?}", self.status)
    }
}

fn run_command_with_timeout(mut cmd: Command, timeout: Duration) -> DockerCommandResult {
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) => {
            return DockerCommandResult {
                stdout: String::new(),
                stderr: String::new(),
                status: None,
                timed_out: false,
                spawn_error: Some(e.to_string()),
            };
        }
    };

    let start = Instant::now();
    let mut timed_out = false;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() >= timeout => {
                timed_out = true;
                let _ = child.kill();
                break;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(40)),
            Err(e) => {
                return DockerCommandResult {
                    stdout: String::new(),
                    stderr: String::new(),
                    status: None,
                    timed_out,
                    spawn_error: Some(e.to_string()),
                };
            }
        }
    }

    match child.wait_with_output() {
        Ok(output) => DockerCommandResult {
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            status: output.status.code(),
            timed_out,
            spawn_error: None,
        },
        Err(e) => DockerCommandResult {
            stdout: String::new(),
            stderr: String::new(),
            status: None,
            timed_out,
            spawn_error: Some(e.to_string()),
        },
    }
}

fn scrolled_for<W: IsA<gtk4::Widget>>(widget: &W) -> gtk4::ScrolledWindow {
    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_child(Some(widget));
    scroll.set_vexpand(true);
    scroll.set_hexpand(true);
    scroll.set_policy(gtk4::PolicyType::Automatic, gtk4::PolicyType::Automatic);
    scroll
}

fn list_row() -> gtk4::ListBoxRow {
    let row = gtk4::ListBoxRow::new();
    row.set_activatable(false);
    row.add_css_class("docker-row");
    row
}

fn clear_list(list: &gtk4::ListBox) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
}

fn append_empty(list: &gtk4::ListBox, message: &str) {
    let row = list_row();
    let label = secondary_label(message);
    label.set_margin_top(12);
    label.set_margin_bottom(12);
    label.set_margin_start(12);
    label.set_margin_end(12);
    row.set_child(Some(&label));
    list.append(&row);
}

fn primary_label(text: &str) -> gtk4::Label {
    let label = gtk4::Label::new(Some(text));
    label.set_halign(gtk4::Align::Start);
    label.set_xalign(0.0);
    label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    label
}

fn secondary_label(text: &str) -> gtk4::Label {
    let label = gtk4::Label::new(Some(text));
    label.add_css_class("dim-label");
    label.add_css_class("caption");
    label.set_halign(gtk4::Align::Start);
    label.set_xalign(0.0);
    label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    label
}

fn health_icon(level: HealthLevel) -> gtk4::Box {
    let container = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    container.add_css_class("docker-health-dot");
    container.add_css_class(match level {
        HealthLevel::Ok => "docker-health-ok",
        HealthLevel::Warn => "docker-health-warn",
        HealthLevel::Critical => "docker-health-critical",
    });
    let icon = match level {
        HealthLevel::Ok => "emblem-ok-symbolic",
        HealthLevel::Warn => "dialog-warning-symbolic",
        HealthLevel::Critical => "dialog-error-symbolic",
    };
    container.append(&gtk4::Image::from_icon_name(icon));
    container
}

fn icon_button(icon: &str, tooltip: &str) -> gtk4::Button {
    let button = gtk4::Button::from_icon_name(icon);
    button.add_css_class("flat");
    button.add_css_class("docker-action-button");
    button.set_tooltip_text(Some(tooltip));
    button
}

fn display_or<'a>(value: &'a str, fallback: &'a str) -> &'a str {
    if value.trim().is_empty() {
        fallback
    } else {
        value
    }
}

fn short_id(id: &str) -> String {
    let clean = id.strip_prefix("sha256:").unwrap_or(id);
    clean.chars().take(12).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn container_health_detects_unhealthy_and_exited() {
        assert_eq!(
            container_health("Up 2 minutes (unhealthy)", "running"),
            HealthLevel::Critical
        );
        assert_eq!(
            container_health("Exited (1) 3 seconds ago", "exited"),
            HealthLevel::Warn
        );
        assert_eq!(container_health("Up 1 hour", "running"), HealthLevel::Ok);
    }

    #[test]
    fn service_health_detects_under_replicated_services() {
        assert_eq!(service_health("3/3"), HealthLevel::Ok);
        assert_eq!(service_health("1/3"), HealthLevel::Warn);
        assert_eq!(service_health("0/3"), HealthLevel::Critical);
        assert_eq!(service_health("global"), HealthLevel::Ok);
    }

    #[test]
    fn shell_quote_preserves_safe_args_and_quotes_templates() {
        assert_eq!(shell_quote("service-name_1"), "service-name_1");
        assert_eq!(
            shell_quote("{{range .Config.Env}}{{println .}}{{end}}"),
            "'{{range .Config.Env}}{{println .}}{{end}}'"
        );
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
    }

    #[test]
    fn compose_projects_parse_json_array() {
        let projects = parse_compose_projects(
            r#"[{"Name":"demo","Status":"running(2)","ConfigFiles":"/tmp/docker-compose.yml"}]"#,
        );
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].name, "demo");
        assert_eq!(projects[0].status, "running(2)");
    }
}
