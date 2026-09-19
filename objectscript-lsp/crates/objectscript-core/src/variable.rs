use crate::parse_structures::{TypeName, Variable, VariableDefType};

impl Variable {
    /// Construct a `Variable` with an optional declared argument type and inferred expression types.
    ///
    /// `arg_type` is typically set for method arguments, while `var_type` represents the inferred
    /// types/atoms observed in the RHS/default expression.
    pub fn new(
        var_name: String,
        arg_type: Option<TypeName>,
        is_public: bool,
        variable_def_type: VariableDefType,
    ) -> Self {
        Self {
            name: var_name,
            arg_type,
            is_public,
            variable_type: variable_def_type,
        }
    }
}
