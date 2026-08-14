## MODIFIED Requirements

### Requirement: Individual failure does not abort the remaining set
The system SHALL attempt approved repairs independently and continue with remaining installations after an individual failure. It SHALL show a terminal result for every selected installation, persist one exact post-action receipt containing every attempted Agent and Skill destination and terminal outcome, retain the existing bounded aggregate summary, and reconcile both ledgers after the operation. The retained results surface SHALL link to that exact Activity receipt.

#### Scenario: One repair fails
- **WHEN** an approved item fails while later items remain
- **THEN** the failed item records its exact destination and bounded error in the receipt and the system continues attempting the remaining approved items

#### Scenario: Repair run completes
- **WHEN** every approved item has reached success or failure
- **THEN** the workflow shows all per-item outcomes, records one destination-exact receipt with matching success and failure counts, displays the final reconciled installation truth, and offers an action to view that receipt in Activity
