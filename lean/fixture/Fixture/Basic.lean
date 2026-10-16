/-! Declarations of every state, for the inspection's tests. -/

namespace Fixture

/-- The double of a number. -/
def double (n : Nat) : Nat := n + n

/-- Doubling is adding a number to itself. -/
theorem double_eq (n : Nat) : double n = n + n := rfl

theorem double_comm (n : Nat) : double n = n + n := by
  rw [double_eq]

/-- Not proved yet. -/
theorem hard (n : Nat) : double n = 2 * n := by
  sorry

/-- A definition whose data is missing. -/
def unknown : Nat := sorry

/-- A definition with a missing proof, but whole data. -/
def half (n : Nat) (h : n % 2 = 0) : Nat :=
  have : n = 2 * (n / 2) := sorry
  n / 2

/-- A pair of numbers. -/
structure Pair where
  fst : Nat
  snd : Nat

def Pair.sum (p : Pair) : Nat :=
  match p with
  | ⟨a, b⟩ => a + b

axiom choice_like : ∀ n : Nat, ∃ m, m = double n

end Fixture
