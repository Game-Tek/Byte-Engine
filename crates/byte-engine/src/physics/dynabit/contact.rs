use math::{Point, UnitVector, collision::DynamicIntersection, collision::Intersection};

/// The `Contact` struct stores the world-space data required to resolve one collision.
#[derive(Debug)]
pub struct Contact {
	pub(crate) a: Side,
	pub(crate) b: Side,
	pub(crate) normal: UnitVector,
	pub(crate) depth: f32,
	/// How far into the step the bodies touch, in seconds. Zero for bodies that touch where they are. For bodies
	/// that touch later, the contact holds them to the speed that brings them together as the step ends.
	pub(crate) toi: f32,
}

impl Contact {
	/// Converts a static math intersection into a physics contact between bodies that touch where they are.
	pub(crate) fn from_static(a: usize, b: usize, intersection: Intersection) -> Self {
		Self {
			a: Side {
				object: a,
				point: intersection.point_on_a(),
			},
			b: Side {
				object: b,
				point: intersection.point_on_b(),
			},
			normal: intersection.normal(),
			depth: intersection.depth(),
			toi: 0.0,
		}
	}

	/// Converts a dynamic math intersection into a physics contact at the time the bodies touch.
	pub(crate) fn from_dynamic(a: usize, b: usize, intersection: DynamicIntersection) -> Self {
		let toi = intersection.toi();
		Self {
			toi,
			..Self::from_static(a, b, intersection.into_contact())
		}
	}
}

/// The `Side` struct identifies one body and its world-space contact point.
#[derive(Debug)]
pub struct Side {
	pub(crate) object: usize,
	pub(crate) point: Point,
}

/// The `Contacts` struct holds a physics step's contacts in the order they were found, as the parts lanes found them
/// in, so that gathering what every lane found moves no contact.
///
/// Fill the parts through [`Self::part_mut`], in the order a single scan would find their contacts, then call
/// [`Self::seal`]. After that a contact's index is its place in that order.
#[derive(Default)]
pub(crate) struct Contacts {
	parts: Vec<Vec<Contact>>,
	/// The index of each part's first contact, followed by the number of contacts. Written by [`Self::seal`].
	starts: Vec<usize>,
}

impl Contacts {
	/// Returns the part at `part`, adding empty parts up to it. Parts keep their storage between steps.
	pub(crate) fn part_mut(&mut self, part: usize) -> &mut Vec<Contact> {
		if self.parts.len() <= part {
			self.parts.resize_with(part + 1, Vec::new);
		}
		&mut self.parts[part]
	}

	/// Numbers the contacts through the parts as they are now. Call it once every part is filled.
	pub(crate) fn seal(&mut self) {
		self.starts.clear();
		let mut start = 0;
		for part in &self.parts {
			self.starts.push(start);
			start += part.len();
		}
		self.starts.push(start);
	}

	/// Returns the number of contacts that were numbered by [`Self::seal`].
	pub(crate) fn len(&self) -> usize {
		self.starts.last().copied().unwrap_or(0)
	}

	/// Returns the contact at `index`.
	#[inline]
	pub(crate) fn get(&self, index: usize) -> &Contact {
		// There are as many parts as lanes, so looking through them takes less than a search would.
		let part = self.starts[1..].iter().take_while(|end| **end <= index).count();
		&self.parts[part][index - self.starts[part]]
	}

	/// Returns the contacts in order.
	pub(crate) fn iter(&self) -> impl Iterator<Item = &Contact> {
		self.parts.iter().flatten()
	}

	/// Empties every part, ready for the next step.
	pub(crate) fn clear(&mut self) {
		for part in &mut self.parts {
			part.clear();
		}
		self.starts.clear();
	}
}

#[cfg(test)]
mod tests {
	use math::{Sphere, collision::sphere_vs_sphere};

	use super::*;

	/// Returns a contact between the bodies in slots `a` and `b`.
	fn contact(a: usize, b: usize) -> Contact {
		let touching = sphere_vs_sphere(
			&Sphere::new(Point::origin(), 1.0),
			&Sphere::new(Point::new(1.0, 0.0, 0.0), 1.0),
		);
		Contact::from_static(a, b, touching.expect("the spheres overlap"))
	}

	#[test]
	fn contacts_are_numbered_through_their_parts_in_order() {
		let mut contacts = Contacts::default();
		// The part between the two others found nothing, as a lane with no overlapping bounds would.
		contacts.part_mut(2).extend([contact(4, 5), contact(6, 7)]);
		contacts.part_mut(0).extend([contact(0, 1), contact(2, 3)]);
		contacts.seal();

		assert_eq!(contacts.len(), 4);
		let firsts = |contacts: &Contacts| {
			(0..contacts.len())
				.map(|index| contacts.get(index).a.object)
				.collect::<Vec<_>>()
		};
		assert_eq!(firsts(&contacts), [0, 2, 4, 6]);
		assert_eq!(
			contacts.iter().map(|contact| contact.b.object).collect::<Vec<_>>(),
			[1, 3, 5, 7]
		);

		contacts.clear();
		contacts.seal();
		assert_eq!(contacts.len(), 0);
	}
}
