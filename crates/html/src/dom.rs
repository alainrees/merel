//! A plain tree for html5ever to build into: an arena of nodes with their element names,
//! attributes and text. Nothing here interprets the document; [`crate::parse`] walks it.

use std::borrow::Cow;
use std::cell::{Ref, RefCell};

use html5ever::interface::{ElemName, ElementFlags, NodeOrText, QuirksMode, TreeSink};
use html5ever::tendril::StrTendril;
use html5ever::{Attribute, LocalName, Namespace, QualName, ns};

pub enum Data {
    Document,
    Element {
        name: QualName,
        attrs: Vec<Attribute>,
    },
    Text(String),
    /// Comments, doctypes and processing instructions: kept for structure, never shown.
    Other,
}

pub struct Node {
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    pub data: Data,
}

/// Node 0 is the document.
pub struct Dom {
    nodes: RefCell<Vec<Node>>,
    /// What `elem_name` answers for a node that is not an element; the tree builder only asks
    /// about elements, so this is a safety net rather than a path that runs.
    no_name: QualName,
}

impl Default for Dom {
    fn default() -> Self {
        let document = Node { parent: None, children: Vec::new(), data: Data::Document };
        Dom { nodes: RefCell::new(vec![document]), no_name: QualName::new(None, ns!(), LocalName::from("")) }
    }
}

impl Dom {
    pub fn nodes(&self) -> Ref<'_, Vec<Node>> {
        self.nodes.borrow()
    }

    fn push(&self, data: Data) -> usize {
        let mut nodes = self.nodes.borrow_mut();
        nodes.push(Node { parent: None, children: Vec::new(), data });
        nodes.len() - 1
    }

    fn detach(nodes: &mut [Node], child: usize) {
        if let Some(parent) = nodes.get(child).and_then(|n| n.parent)
            && let Some(p) = nodes.get_mut(parent)
        {
            p.children.retain(|&c| c != child);
        }
        if let Some(c) = nodes.get_mut(child) {
            c.parent = None;
        }
    }

    /// Appends `text` to the last child of `parent` when that is a text node, else as a new one.
    fn append_text(&self, parent: usize, text: &str) {
        let mut nodes = self.nodes.borrow_mut();
        let last = nodes.get(parent).and_then(|p| p.children.last().copied());
        if let Some(last) = last
            && let Some(Node { data: Data::Text(existing), .. }) = nodes.get_mut(last)
        {
            existing.push_str(text);
            return;
        }
        nodes.push(Node { parent: Some(parent), children: Vec::new(), data: Data::Text(text.to_string()) });
        let id = nodes.len() - 1;
        if let Some(p) = nodes.get_mut(parent) {
            p.children.push(id);
        }
    }

    fn insert(&self, parent: usize, at: Option<usize>, child: NodeOrText<usize>) {
        match child {
            NodeOrText::AppendText(text) => {
                // Text before a sibling is rare (foster parenting); it becomes a node of its own.
                match at {
                    None => self.append_text(parent, &text),
                    Some(_) => {
                        let id = self.push(Data::Text(text.to_string()));
                        self.insert(parent, at, NodeOrText::AppendNode(id));
                    }
                }
            }
            NodeOrText::AppendNode(id) => {
                let mut nodes = self.nodes.borrow_mut();
                Self::detach(&mut nodes, id);
                if let Some(n) = nodes.get_mut(id) {
                    n.parent = Some(parent);
                }
                if let Some(p) = nodes.get_mut(parent) {
                    match at.and_then(|s| p.children.iter().position(|&c| c == s)) {
                        Some(i) => p.children.insert(i, id),
                        None => p.children.push(id),
                    }
                }
            }
        }
    }
}

/// The name of one element, borrowed from the arena for as long as the tree builder looks.
pub struct Name<'a> {
    nodes: Ref<'a, Vec<Node>>,
    id: usize,
    no_name: &'a QualName,
}

impl Name<'_> {
    fn qual(&self) -> &QualName {
        match self.nodes.get(self.id).map(|n| &n.data) {
            Some(Data::Element { name, .. }) => name,
            _ => self.no_name,
        }
    }
}

impl std::fmt::Debug for Name<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.qual())
    }
}

impl ElemName for Name<'_> {
    fn ns(&self) -> &Namespace {
        &self.qual().ns
    }

    fn local_name(&self) -> &LocalName {
        &self.qual().local
    }
}

impl TreeSink for Dom {
    type Handle = usize;
    type Output = Dom;
    type ElemName<'a> = Name<'a>;

    fn finish(self) -> Dom {
        self
    }

    fn parse_error(&self, _msg: Cow<'static, str>) {}

    fn get_document(&self) -> usize {
        0
    }

    fn elem_name<'a>(&'a self, target: &'a usize) -> Name<'a> {
        Name { nodes: self.nodes.borrow(), id: *target, no_name: &self.no_name }
    }

    fn create_element(&self, name: QualName, attrs: Vec<Attribute>, flags: ElementFlags) -> usize {
        let id = self.push(Data::Element { name, attrs });
        if flags.template {
            // Template contents are never shown; they get an empty holder so the builder
            // has somewhere to put them.
            let holder = self.push(Data::Other);
            if let Some(n) = self.nodes.borrow_mut().get_mut(id) {
                n.children.push(holder);
            }
        }
        id
    }

    fn create_comment(&self, _text: StrTendril) -> usize {
        self.push(Data::Other)
    }

    fn create_pi(&self, _target: StrTendril, _data: StrTendril) -> usize {
        self.push(Data::Other)
    }

    fn append(&self, parent: &usize, child: NodeOrText<usize>) {
        self.insert(*parent, None, child);
    }

    fn append_based_on_parent_node(&self, element: &usize, prev_element: &usize, child: NodeOrText<usize>) {
        let has_parent = self.nodes.borrow().get(*element).is_some_and(|n| n.parent.is_some());
        if has_parent { self.append_before_sibling(element, child) } else { self.append(prev_element, child) }
    }

    fn append_doctype_to_document(&self, _name: StrTendril, _public_id: StrTendril, _system_id: StrTendril) {}

    fn get_template_contents(&self, target: &usize) -> usize {
        self.nodes.borrow().get(*target).and_then(|n| n.children.first().copied()).unwrap_or(*target)
    }

    fn same_node(&self, x: &usize, y: &usize) -> bool {
        x == y
    }

    fn set_quirks_mode(&self, _mode: QuirksMode) {}

    fn append_before_sibling(&self, sibling: &usize, new_node: NodeOrText<usize>) {
        let parent = self.nodes.borrow().get(*sibling).and_then(|n| n.parent);
        match parent {
            Some(parent) => self.insert(parent, Some(*sibling), new_node),
            None => self.insert(0, None, new_node),
        }
    }

    fn add_attrs_if_missing(&self, target: &usize, new: Vec<Attribute>) {
        if let Some(Node { data: Data::Element { attrs, .. }, .. }) = self.nodes.borrow_mut().get_mut(*target) {
            for a in new {
                if !attrs.iter().any(|x| x.name == a.name) {
                    attrs.push(a);
                }
            }
        }
    }

    fn remove_from_parent(&self, target: &usize) {
        Self::detach(&mut self.nodes.borrow_mut(), *target);
    }

    fn reparent_children(&self, node: &usize, new_parent: &usize) {
        let mut nodes = self.nodes.borrow_mut();
        let children = nodes.get_mut(*node).map(|n| std::mem::take(&mut n.children)).unwrap_or_default();
        for &c in &children {
            if let Some(n) = nodes.get_mut(c) {
                n.parent = Some(*new_parent);
            }
        }
        if let Some(p) = nodes.get_mut(*new_parent) {
            p.children.extend(children);
        }
    }
}
