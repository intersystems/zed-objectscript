use crate::parse_structures::{ReturnType, Variable};

impl Variable {
    /// Construct a `Variable` with an optional declared argument type and inferred expression types.
    ///
    /// `arg_type` is typically set for method arguments, while `var_type` represents the inferred
    /// types/atoms observed in the RHS/default expression.
    pub fn new(
        var_name: String,
        arg_type: Option<ReturnType>,
        is_public: bool,
        is_oref: bool,
        cls: Option<String>,
    ) -> Self {
        Self {
            name: var_name,
            arg_type,
            is_public,
            is_oref,
            cls,
        }
    }
}
