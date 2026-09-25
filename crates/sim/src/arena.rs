//! Typed slot arena with generational IDs (issue #5). Iteration is in slot order and the
//! free list is a stack, so allocation and iteration are deterministic and roll back with
//! the state.

use serde::{Deserialize, Serialize};
use std::marker::PhantomData;

/// Handle to a value in an [`Arena<T>`]. Stale once the value is removed: the slot's
/// generation moves on, so a reused slot never answers to an old ID.
pub struct Id<T> {
    index: usize,
    generation: u32,
    _type: PhantomData<fn() -> T>,
}

// Manual impls: derives would demand the same traits of `T`, which the handle never holds.
impl<T> Clone for Id<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for Id<T> {}
impl<T> PartialEq for Id<T> {
    fn eq(&self, other: &Self) -> bool {
        (self.index, self.generation) == (other.index, other.generation)
    }
}
impl<T> Eq for Id<T> {}
impl<T> std::fmt::Debug for Id<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Id({}v{})", self.index, self.generation)
    }
}

impl<T> Id<T> {
    const fn new(index: usize, generation: u32) -> Self {
        Self {
            index,
            generation,
            _type: PhantomData,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
struct Slot<T> {
    generation: u32,
    value: Option<T>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Arena<T> {
    slots: Vec<Slot<T>>,
    /// Freed slot indices; the most recently freed is reused first.
    free: Vec<usize>,
}

impl<T> Default for Arena<T> {
    fn default() -> Self {
        Self {
            slots: Vec::new(),
            free: Vec::new(),
        }
    }
}

impl<T> Arena<T> {
    pub fn insert(&mut self, value: T) -> Id<T> {
        if let Some(index) = self.free.pop()
            && let Some(slot) = self.slots.get_mut(index)
        {
            slot.value = Some(value);
            return Id::new(index, slot.generation);
        }
        let index = self.slots.len();
        self.slots.push(Slot {
            generation: 0,
            value: Some(value),
        });
        Id::new(index, 0)
    }

    #[must_use]
    pub fn get(&self, id: Id<T>) -> Option<&T> {
        self.slots
            .get(id.index)
            .filter(|s| s.generation == id.generation)
            .and_then(|s| s.value.as_ref())
    }

    /// Live values in slot order.
    pub fn iter(&self) -> impl Iterator<Item = (Id<T>, &T)> {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(index, s)| s.value.as_ref().map(|v| (Id::new(index, s.generation), v)))
    }

    /// Live values in slot order.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (Id<T>, &mut T)> {
        self.slots
            .iter_mut()
            .enumerate()
            .filter_map(|(index, s)| s.value.as_mut().map(|v| (Id::new(index, s.generation), v)))
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.slots.iter().filter(|s| s.value.is_some()).count()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Visits live values in slot order and removes those `keep` rejects.
    pub fn retain(&mut self, mut keep: impl FnMut(Id<T>, &mut T) -> bool) {
        for (index, slot) in self.slots.iter_mut().enumerate() {
            if let Some(value) = &mut slot.value
                && !keep(Id::new(index, slot.generation), value)
            {
                slot.value = None;
                slot.generation = slot.generation.wrapping_add(1);
                self.free.push(index);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removed_ids_go_stale_and_their_slot_is_reused() {
        let mut arena = Arena::default();
        let a = arena.insert('a');
        let b = arena.insert('b');
        arena.retain(|id, _| id != a);
        assert_eq!(arena.get(a), None);
        let c = arena.insert('c');
        assert_eq!(
            (c.index, arena.get(a), arena.get(c)),
            (a.index, None, Some(&'c'))
        );
        let order: Vec<_> = arena.iter().map(|(id, &v)| (id, v)).collect();
        assert_eq!(order, [(c, 'c'), (b, 'b')]);
    }
}
