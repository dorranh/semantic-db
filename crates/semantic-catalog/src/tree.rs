//! Canonical persistent treap: O(log n) expected path copying and root hashing.
//! Key-derived priorities make content-identical trees independent of insertion order.
use crate::SnapshotRelation;
use sha2::{Digest, Sha256};
use std::sync::Arc;
pub(crate) trait Revisioned {
    fn revision(&self) -> &str;
}
impl Revisioned for SnapshotRelation {
    fn revision(&self) -> &str {
        &self.reference().revision
    }
}
#[derive(Debug)]
pub(crate) struct Root<T = SnapshotRelation>(Option<Arc<Node<T>>>);
impl<T> Clone for Root<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}
impl<T> Default for Root<T> {
    fn default() -> Self {
        Self(None)
    }
}
#[derive(Debug)]
struct Node<T> {
    key: String,
    value: Arc<T>,
    priority: [u8; 32],
    left: Root<T>,
    right: Root<T>,
    size: usize,
    digest: String,
}
impl<T: Revisioned> Root<T> {
    pub fn len(&self) -> usize {
        self.0.as_ref().map_or(0, |n| n.size)
    }
    pub fn digest(&self) -> &str {
        self.0
            .as_ref()
            .map_or("semantic-catalog-v2-empty", |n| n.digest.as_str())
    }
    pub fn get(&self, key: &str) -> Option<&Arc<T>> {
        let mut current = self.0.as_deref();
        while let Some(node) = current {
            match key.cmp(&node.key) {
                std::cmp::Ordering::Less => current = node.left.0.as_deref(),
                std::cmp::Ordering::Greater => current = node.right.0.as_deref(),
                std::cmp::Ordering::Equal => return Some(&node.value),
            }
        }
        None
    }
    fn node(key: String, value: Arc<T>, left: Self, right: Self) -> Self {
        let priority = Sha256::digest(key.as_bytes()).into();
        let size = 1 + left.len() + right.len();
        let mut hash = Sha256::new();
        hash.update(b"semantic-catalog-merkle-v2");
        for part in [left.digest(), &key, value.revision(), right.digest()] {
            hash.update((part.len() as u64).to_be_bytes());
            hash.update(part.as_bytes());
        }
        let digest = format!("{:x}", hash.finalize());
        Self(Some(Arc::new(Node {
            key,
            value,
            priority,
            left,
            right,
            size,
            digest,
        })))
    }
    pub fn insert(&self, key: String, value: Arc<T>) -> Self {
        let Some(node) = self.0.as_ref() else {
            return Self::node(key, value, Self::default(), Self::default());
        };
        match key.cmp(&node.key) {
            std::cmp::Ordering::Equal => {
                Self::node(key, value, node.left.clone(), node.right.clone())
            }
            std::cmp::Ordering::Less => {
                let left = node.left.insert(key, value);
                let child = left.0.as_ref().expect("inserted node");
                if (&child.priority, &child.key) < (&node.priority, &node.key) {
                    let right = Self::node(
                        node.key.clone(),
                        node.value.clone(),
                        child.right.clone(),
                        node.right.clone(),
                    );
                    Self::node(
                        child.key.clone(),
                        child.value.clone(),
                        child.left.clone(),
                        right,
                    )
                } else {
                    Self::node(
                        node.key.clone(),
                        node.value.clone(),
                        left,
                        node.right.clone(),
                    )
                }
            }
            std::cmp::Ordering::Greater => {
                let right = node.right.insert(key, value);
                let child = right.0.as_ref().expect("inserted node");
                if (&child.priority, &child.key) < (&node.priority, &node.key) {
                    let left = Self::node(
                        node.key.clone(),
                        node.value.clone(),
                        node.left.clone(),
                        child.left.clone(),
                    );
                    Self::node(
                        child.key.clone(),
                        child.value.clone(),
                        left,
                        child.right.clone(),
                    )
                } else {
                    Self::node(
                        node.key.clone(),
                        node.value.clone(),
                        node.left.clone(),
                        right,
                    )
                }
            }
        }
    }
    pub fn remove(&self, key: &str) -> Self {
        let Some(node) = self.0.as_ref() else {
            return self.clone();
        };
        match key.cmp(&node.key) {
            std::cmp::Ordering::Equal => Self::merge(&node.left, &node.right),
            std::cmp::Ordering::Less => Self::node(
                node.key.clone(),
                node.value.clone(),
                node.left.remove(key),
                node.right.clone(),
            ),
            std::cmp::Ordering::Greater => Self::node(
                node.key.clone(),
                node.value.clone(),
                node.left.clone(),
                node.right.remove(key),
            ),
        }
    }
    fn merge(left: &Self, right: &Self) -> Self {
        match (&left.0, &right.0) {
            (None, _) => right.clone(),
            (_, None) => left.clone(),
            (Some(l), Some(r)) if (&l.priority, &l.key) < (&r.priority, &r.key) => Self::node(
                l.key.clone(),
                l.value.clone(),
                l.left.clone(),
                Self::merge(&l.right, right),
            ),
            (Some(_), Some(r)) => Self::node(
                r.key.clone(),
                r.value.clone(),
                Self::merge(left, &r.left),
                r.right.clone(),
            ),
        }
    }
    pub fn iter(&self) -> Iter<'_, T> {
        let mut iter = Iter { stack: Vec::new() };
        iter.descend(self.0.as_deref());
        iter
    }
}
pub(crate) struct Iter<'a, T> {
    stack: Vec<&'a Node<T>>,
}
impl<'a, T> Iter<'a, T> {
    fn descend(&mut self, mut node: Option<&'a Node<T>>) {
        while let Some(value) = node {
            self.stack.push(value);
            node = value.left.0.as_deref();
        }
    }
}
impl<'a, T> Iterator for Iter<'a, T> {
    type Item = &'a Arc<T>;
    fn next(&mut self) -> Option<Self::Item> {
        let node = self.stack.pop()?;
        self.descend(node.right.0.as_deref());
        Some(&node.value)
    }
}
