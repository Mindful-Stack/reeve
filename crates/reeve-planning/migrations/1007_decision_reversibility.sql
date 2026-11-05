-- kb/ADR-0013: a planner-side reversibility, and what each close said and through which token kind. NULL
-- reversibility is unset (behaves as two-way); NULL closed_kind/closed_via on open rows and on closes before this.
ALTER TABLE required_decisions
  ADD COLUMN reversibility TEXT CHECK (reversibility IN ('one_way','two_way')),
  ADD COLUMN closed_kind TEXT CHECK (closed_kind IN ('decided','withdrawn')),
  ADD COLUMN closed_via TEXT CHECK (closed_via IN ('interactive','automation'));
