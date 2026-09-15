/**
 * Compile-time guards for the file-trust decisions that must never be implicit.
 *
 * Nothing imports this and nothing runs it: it exists so `npm run check` fails if one of the
 * signatures below stops rejecting the shape underneath it. Unit tests cannot cover this, because
 * the thing being asserted is that certain calls do not compile, and `src/**\/*.test.ts` is
 * excluded from the type-check anyway.
 *
 * Every `@ts-expect-error` here is load-bearing in both directions. TypeScript reports an unused
 * directive as an error, so if a permissive default comes back and the call below starts
 * compiling, this file breaks the build rather than quietly passing.
 */
import {
  DEFAULT_FILE_TRUST_POLICY, mayAutoLoadFile, type PassiveFileClass,
} from "./file-trust.ts";

const policy = DEFAULT_FILE_TRUST_POLICY;

// SEC-MEDIA-002. The classification decides whether attacker-chosen bytes reach the platform
// media stack with no user gesture. Omitting it must not compile to the permissive answer.
// @ts-expect-error the passive file classification must be explicit
mayAutoLoadFile(policy, "member", true);

// A bare boolean was the old shape. `true` at a call site reads as nothing, and the two spellings
// would silently disagree about which value meant media, so the boolean must not compile either.
// @ts-expect-error the passive file classification is a PassiveFileClass, not a boolean
mayAutoLoadFile(policy, "member", true, true);

// Only the two named classes exist. A near-miss string is a typo that would otherwise resolve to
// "not media" and fail closed, which is safe but silent; make it a build error instead.
// @ts-expect-error "media" is not one of the PassiveFileClass values
mayAutoLoadFile(policy, "member", true, "media");

// The permitted calls, so this file also fails if the real signature is narrowed by accident.
const admitted: PassiveFileClass = "validated-media";
const withheld: PassiveFileClass = "non-media";
void mayAutoLoadFile(policy, "member", true, admitted);
void mayAutoLoadFile(policy, "member", true, withheld);
