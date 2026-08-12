Only these rules. Do not invent others, and do not judge against rules not listed here.

1. ONLY PROJECT-SPECIFIC KNOWLEDGE. Every sentence must teach something learnable only from
   this codebase. Cut what a competent engineer already brought with them or can look up:
   platform, protocol, language or domain behaviour that a textbook, a man page or a spec
   already teaches. Test each sentence: could it stand, unchanged, in the manual or README of
   software this project did not write? Then REVISE quoting it — naming that software's fields
   or states precisely does not make it ours, and neither does arguing for a choice made here.
   A sentence about code in THIS repository — its reader, its loop, its parser, its config keys
   and invariants, and how any of them behaves HERE — could not stand anywhere else, and is
   never a violation of this rule however ordinary the mechanism around it looks.

2. NO ALLUSION TO AN OCCASION. Nothing that requires having been there: "the incident", "the
   outage", "last round", or any definite reference to an event the file does not describe.
   State the mechanism that makes the rule true, not the occasion that taught it. A passage
   naming no event is never a violation of this rule, however much it speaks of time.

3. NO NARRATION OF A PREVIOUS STATE. A violation says this project was once different —
   "used to be", "previously", "an earlier version", "no longer" — or opens by setting a
   scene out of earlier behaviour. Ordering inside the design, and naming an alternative it
   does not take, are the decision itself and not narration.

4. NO ENUMERATED VALUES, AND NO HOW THE CODE CAN STATE ITSELF. No lists of fields, columns,
   enum variants, wire codes, counts, widths or timeouts — those live in the code and go
   stale when one is renamed. Name the category instead. Likewise a passage that walks
   through how something works, which a later code change would falsify, belongs in the code;
   only how-with-why-attached earns its place.

5. NO SPEC RESTATEMENT. Paraphrasing an RFC, grammar, character set or MAY/MUST teaches
   nothing. Only a choice the spec left open, and closed one way here, is a decision.
