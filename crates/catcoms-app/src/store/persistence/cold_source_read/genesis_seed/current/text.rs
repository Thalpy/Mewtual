//! Sealed call-throughs to each actual family's original node.
//!
//! No callback lends the node/SDK/RNG and no extractor can adopt it. The operational host
//! supplies actual selected-source/index/signing admission separately. These methods check
//! cached original ownership; they deliberately never invoke strict genesis physical proof.
use super::UnexposedV2GenesisCurrent;

macro_rules! node_call_throughs {
    ($ty:ident) => {
        impl<T: catcoms_rt::MeshTransport, R: catcoms_rt::CryptoRngCore> $ty<T, R> {
            fn require_text_original_data(
                &self,
                store: &crate::store::ServerStore,
            ) -> Result<(), catcoms_sync::owner_v2::V2CreateRefusal> {
                use catcoms_sync::owner_v2::V2CreateRefusal;
                let owner = self.text_owner_ref().map_err(|_| V2CreateRefusal::Source)?;
                let writer = owner.observe_writer();
                owner.require_original_negative(store, &writer)
                    .map_err(|_| V2CreateRefusal::Source)
            }

            /// Outside every global writer. The SourceRef created by the first check
            /// ends when that helper RETURNS, before mutating this SAME node's RNG.
            /// The caller's retained slot owns partial/materialized DATA even on Err.
            pub(in crate::store::persistence::cold_source_read::genesis_seed::current)
            fn prepare_text_create_data(
                &mut self,
                store: &crate::store::ServerStore,
                input: catcoms_sync::owner_v2::V2CreateRequest<'_>,
                slot: &mut Option<catcoms_sync::owner_v2::PreparedV2CreateData>,
            ) -> Result<(), catcoms_sync::owner_v2::V2CreateRefusal> {
                self.require_text_original_data(store)?;
                let result = self.node.prepare_create_data(input, slot);
                self.require_text_original_data(store)?;
                result
            }

            /// Outside global writers; full parent validation is intentionally not the
            /// short key-use cut. Actual model/payment remains in Store pending custody.
            pub(in crate::store::persistence::cold_source_read::genesis_seed::current)
            fn prepare_text_append_data(
                &self,
                store: &crate::store::ServerStore,
                model: &catcoms_replication::chat::owner_state::transition::ReplayMath,
                action: &catcoms_replication::chat::SignedAction,
                slot: &mut Option<catcoms_sync::owner_v2::PreparedV2AppendData>,
            ) -> Result<(), catcoms_sync::owner_v2::V2CreateRefusal> {
                self.require_text_original_data(store)?;
                let result = self.node.prepare_create_append_data(model, action, slot);
                self.require_text_original_data(store)?;
                result
            }

            /// Full preparation/validation before the global-writer signing admission.
            /// The observation contains the actual original opaque plan, not caller bytes.
            pub(in crate::store::persistence::cold_source_read::genesis_seed::current)
            fn observe_text_append_plan(
                &self,
                store: &crate::store::ServerStore,
                model: &catcoms_replication::chat::owner_state::transition::ReplayMath,
                prepared: &catcoms_sync::owner_v2::PreparedV2AppendData,
            ) -> Result<catcoms_sync::owner_v2::V2AppendPlanObservation,
                        catcoms_sync::owner_v2::V2CreateRefusal> {
                self.require_text_original_data(store)?;
                let result = prepared.observe_plan_data(&self.node, model);
                self.require_text_original_data(store)?;
                result
            }

            /// Root's actual selected-I/current/close guard wraps this family-only call.
            /// The node compares SAME opaque plan and stores raw64 before late negatives;
            /// no Header verification, derivation, encoding or RNG is performed here.
            pub(in crate::store::persistence::cold_source_read::genesis_seed::current)
            fn sign_text_append_bytes(
                &self,
                store: &crate::store::ServerStore,
                original_plan: &catcoms_sync::owner_v2::V2AppendPlanObservation,
                prepared: &mut catcoms_sync::owner_v2::PreparedV2AppendData,
            ) -> Result<(), catcoms_sync::owner_v2::V2CreateRefusal> {
                self.require_text_original_data(store)?;
                let result = self.node.sign_reserved_append_bytes_data(original_plan, prepared);
                self.require_text_original_data(store)?;
                result
            }

            /// Outside global writers. Reconstruct and verify Header from existing raw;
            /// the paired Core verifier derives the candidate exactly ONCE afterwards.
            pub(in crate::store::persistence::cold_source_read::genesis_seed::current)
            fn materialize_text_append(
                &self,
                store: &crate::store::ServerStore,
                model: &catcoms_replication::chat::owner_state::transition::ReplayMath,
                prepared: &mut catcoms_sync::owner_v2::PreparedV2AppendData,
            ) -> Result<(), catcoms_sync::owner_v2::V2CreateRefusal> {
                self.require_text_original_data(store)?;
                let result = self.node.materialize_signed_append_data(model, prepared);
                self.require_text_original_data(store)?;
                result
            }

            /// Borrow only bounded unsigned DATA for the actual private I candidate.
            /// No node/SDK/Clock/RNG or signing callback crosses this boundary.
            pub(in crate::store::persistence::cold_source_read::genesis_seed::current)
            fn borrow_unsigned_text_append<'a>(
                &'a self,
                store: &crate::store::ServerStore,
                model: &'a catcoms_replication::chat::owner_state::transition::ReplayMath,
                prepared: &'a catcoms_sync::owner_v2::PreparedV2AppendData,
            ) -> Result<catcoms_sync::owner_v2::V2UnsignedAppendDataView<'a>,
                        catcoms_sync::owner_v2::V2CreateRefusal> {
                self.require_text_original_data(store)?;
                prepared.borrow_unsigned_data(&self.node, model)
            }

            /// ORDINARY-member seams on the SAME original node. None reaches the founder
            /// Append/Header lane. Pre-admission: the full retained device key.
            pub(in crate::store::persistence::cold_source_read::genesis_seed::current)
            fn text_ordinary_member_signer(
                &self,
                store: &crate::store::ServerStore,
            ) -> Result<[u8; 32], catcoms_sync::owner_v2::V2MemberSignRefusal> {
                use catcoms_sync::owner_v2::V2MemberSignRefusal;
                self.require_text_original_data(store)
                    .map_err(|_| V2MemberSignRefusal::Source)?;
                self.node.ordinary_member_signer_data()
            }

            /// Outside every writer: the request's one Clock/RNG draw through this node's
            /// retained seams. The caller's slot caches each value before late checks.
            pub(in crate::store::persistence::cold_source_read::genesis_seed::current)
            fn draw_text_ordinary_inputs(
                &mut self,
                store: &crate::store::ServerStore,
                slot: &mut catcoms_sync::owner_v2::V2MemberDraftInputs,
            ) -> Result<(u64, [u8; 16]), catcoms_sync::owner_v2::V2MemberSignRefusal> {
                use catcoms_sync::owner_v2::V2MemberSignRefusal;
                self.require_text_original_data(store)
                    .map_err(|_| V2MemberSignRefusal::Source)?;
                let result = self.node.draw_ordinary_draft_inputs(slot);
                self.require_text_original_data(store)
                    .map_err(|_| V2MemberSignRefusal::Source)?;
                result
            }

            /// Inside the host's Current/group guard, with Core's original reader held by
            /// `guard`. The caller ran the full original-owner negatives (via
            /// `text_ordinary_member_signer`) inside this SAME Current callback just before
            /// Core admission, so nothing but the node's own key cut runs here. raw64 is
            /// cached in the Plan by the node itself, so no late check can lose it. `inputs` is
            /// the request's draw cell: the node refuses a request drawn on another seed.
            pub(in crate::store::persistence::cold_source_read::genesis_seed::current)
            fn sign_text_ordinary_admitted<'a>(
                &self,
                inputs: &catcoms_sync::owner_v2::V2MemberDraftInputs,
                guard: catcoms_replication::chat::V2OrdinarySigningGuard<'a>,
            ) -> Result<
                catcoms_replication::chat::V2OrdinarySigningGuard<'a>,
                catcoms_sync::owner_v2::V2MemberSignRefusal,
            > {
                self.node.sign_ordinary_admitted(inputs, guard)
            }

            /// Concrete native edge on the SAME family/node; caller retains
            /// the native pins and Ends outside Current/Core guards.
            pub(in crate::store::persistence::cold_source_read::genesis_seed::current)
            fn sign_text_ordinary_admitted_native<'a>(
                &self,
                inputs: &catcoms_sync::owner_v2::V2MemberDraftInputs,
                guard: catcoms_replication::chat::V2OrdinarySigningGuard<'a>,
                original: &catcoms_rt::NativeChatSourceObservation,
                cut: &mut catcoms_rt::NativeChatPreparedKeyCut,
                ends: &mut catcoms_rt::NativeChatKeyEnds,
            ) -> Result<catcoms_replication::chat::V2OrdinarySigningGuard<'a>,
                catcoms_sync::owner_v2::V2MemberSignRefusal> {
                self.node.sign_ordinary_admitted_native(inputs, guard, original, cut, ends)
            }
        }
    };
}

pub(super) use node_call_throughs;
node_call_throughs!(UnexposedV2GenesisCurrent);
