Only these rules. Do not invent others, and do not judge against rules not listed here.

1. ONLY PROJECT-SPECIFIC KNOWLEDGE. Every sentence must teach something learnable only from
   this codebase. Cut what a competent engineer already brought with them: platform, protocol,
   language or domain behaviour that a textbook or a man page already teaches. A sentence
   naming this project's own types, fields, components or invariants is project-specific by
   definition and is never a violation of this rule.

2. NO ALLUSION TO AN OCCASION. Nothing that requires having been there: "the incident", "the
   outage", "last round", or any definite reference to an event the file does not describe.
   State the mechanism that makes the rule true, not the occasion that taught it.

3. NO BEFORE/AFTER NARRATION. Describe the design as it now stands. No "used to be",
   "previously", "an earlier version", "the parser changed this", no scene-setting first
   sentence, no naming the alternative that was rejected as a narrative.

4. NO DERIVABLE CONSEQUENCE. Never state a cost, consequence or tradeoff that follows from
   the mechanism just described. Only the RESPONSE to a consequence earns a line.

5. NO ENUMERATED VALUES, AND NO HOW THE CODE CAN STATE ITSELF. No lists of fields, columns,
   enum variants, wire codes, counts, widths or timeouts — those live in the code and go
   stale when one is renamed. Name the category instead. Likewise a passage that walks
   through how something works, which a later code change would falsify, belongs in the code;
   only how-with-why-attached earns its place.

6. NO SPEC RESTATEMENT. Paraphrasing an RFC, grammar, character set or MAY/MUST teaches
   nothing. Only a choice the spec left open, and closed one way here, is a decision.
