use crate::parse_structures::MethodRef;
use tower_lsp::lsp_types::Url;
use tree_sitter::Range;

/// The Key into `ScopeTree::scopes` representing a single `Scope`.
#[derive(Copy, Hash, Eq, PartialEq, Clone, Debug)]
pub struct ScopeId(pub usize);

/// A variable definition symbol for a public variable (definition site + dependency metadata).
#[derive(Clone, Debug)]
pub struct VariableGlobalSymbol {
    /// Document URl containing the variable definition.
    pub url: Url,
    /// Source range of the variable definition.
    pub location: Range,
    /// Names of other variables referenced by this definition.
    pub var_dependencies: Vec<String>,
}

/// A parameter definition symbol for a parameter.
#[derive(Clone, Debug)]
pub struct ParameterSymbol {
    /// parameter Name
    pub name: String,
    /// Document URl containing the parameter definition.
    pub url: Url,
    /// Source range of the parameter definition.
    pub location: Range,
    /// Ranges of references/uses associated with this symbol.
    pub references: Vec<Range>,
}

/// A property definition symbol for a property.
#[derive(Clone, Debug)]
pub struct PropertySymbol {
    /// Property Name
    pub name: String,
    /// Document URl containing the property definition.
    pub url: Url,
    /// Source range of the property definition.
    pub location: Range,
    /// Ranges of references/uses associated with this symbol.
    pub references: Vec<Range>,
}

/// A class definition symbol (definition site + liveness flag).
#[derive(Clone, Debug)]
pub struct ClassGlobalSymbol {
    /// Class Name
    pub name: String,
    /// Document URl containing the class definition.
    pub url: Url,
    /// Source range of the class definition.
    pub location: Range,
    /// Whether this symbol currently represents a live document/class (false after removal).
    pub alive: bool,
}

/// A private variable symbol (definition + references + dependency metadata).
#[derive(Clone, Debug)]
pub struct VariableSymbol {
    /// Source range of the variable definition.
    pub location: Range,
    /// Source ranges of references/uses associated with this symbol.
    pub references: Vec<Range>,
    /// Names of other variables referenced by this definition.
    pub var_dependencies: Vec<String>,
}

/// A method definition symbol for an ObjectScript method.
#[derive(Clone, Debug)]
pub struct MethodSymbol {
    /// Method name
    pub name: String,
    /// Source range of the method definition.
    pub location: Range,
    /// Ranges of references/uses associated with this symbol.
    pub references: Vec<(Url, Range)>,
    /// Ranges of dependencies associated with this symbol.
    pub method_dependencies: Vec<MethodRef>,
    /// Document URl containing the method definition.
    pub url: Url,
}
