use crate::scope_structures::ScopeId;
use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::hash::Hasher;
use tree_sitter::Range;
/// Stores the Index into `GlobalSemanticModel::classes`.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct ClassId(pub usize);

/// Stores the Method Index, which is assigned by `class.get_next_method_id()`.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct MethodId(pub usize);

/// Stores the Index into the per-class public variable vec in `GlobalSemanticModel::variables::ClassId`, where ClassId represents the class the variable is defined in.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct PublicVarId(pub usize);

/// Stores the Index into `LocalSemanticModel::variables`.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct PrivateVarId(pub usize);

/// Stores the Property Index, which is assigned by `class.get_next_property_id()`.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct PropertyId(pub usize);

/// Stores the Parameter Index, which is assigned by `class.get_next_parameter_id()`.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct ParameterId(pub usize);

/// Differentiates the kind of class member an identifier node represents.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemberType {
    Class,
    ClassDef,
    ClassDefRange,
    Relationship,
    Foreignkey,
    RelativeParameter,
    ParameterDef,
    OrefParameter,
    Projection,
    Index,
    Xdata,
    Storage,
    ClassMethodCall,
    RelativeMethodCall,
    Query,
    Trigger,
    RelativeProperty,
    OrefProperty,
    PropertyDef,
    OrefMethod,
    RoutineMethodCall,
    Routine,
    LocalVariable,
    SystemMember,
    GlobalVariable,
    MethodDef,
    Keyword,
    Procedure,
    DottedStatementTag,
}

/// DFS visitation state.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum DfsState {
    Unvisited,
    Visiting,
    Done,
}

/// Reference to a method implementation in a class.
#[derive(Copy, Clone, Debug)]
pub struct MethodRef {
    pub class: ClassId,
    pub id: MethodId,
    pub offset: Option<usize>,
}

/// Unresolved Reference to a method implementation in a class.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct UnresolvedMethodRef {
    pub class: String,  // unresolved class name
    pub method: String, // unresolved method name
    pub offset: Option<usize>,
    pub method_call_range: Range,
}

/// Reference to a parameter in a class.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct ParameterRef {
    pub class: ClassId,
    pub id: ParameterId,
}

/// Reference to a property in a class.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct PropertyRef {
    pub class: ClassId,
    pub id: PropertyId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Parameter {
    /// If true, the Parameter cannot be overwritten by subclasses.
    pub is_final: Option<bool>,
    /// Parameter Name.
    pub name: String,
    /// Expected return type.
    pub return_type: Option<TypeName>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Property {
    /// Whether property is required or not.
    pub required: bool,
    /// Whether property is public or not.
    pub is_public: bool,
    /// If true, the property cannot be overwritten by subclasses.
    pub is_final: Option<bool>,
    /// Property Name.
    pub name: String,
    /// Whether property is multidimensional or not.
    pub multidimensional: bool,
    /// Expected return type.
    pub return_type: Option<TypeName>,
}

impl PartialEq for MethodRef {
    fn eq(&self, other: &Self) -> bool {
        self.class == other.class && self.id == other.id
        // offset intentionally ignored
    }
}

impl Eq for MethodRef {}

impl Hash for MethodRef {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.class.hash(state);
        self.id.hash(state);

        // offset intentionally ignored
    }
}

// TODO: UNIMPLEMENTED: foreignkey, relationships, storage, query, index, trigger, xdata, projection
/// Semantic representation of a parsed ObjectScript class.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Class {
    /// Class Name.
    pub name: String,
    /// Imported classes referenced by this class.
    pub imports: Vec<String>, // list of class names
    // format: Include (macro file name) ex: include hannah for macro file hannah.inc
    // pub include: Vec<String>, // include files are inherited by subclasses, include files bring in macros at compile time
    // pub include_gen: Vec<String>, // this specifies include files to be generated
    // if inheritance keyword == left, leftmost supersedes all (default)
    // if inheritancedirection == right, right supersedes
    /// Direct parent classes in the `Extends` list.
    pub inherited_classes: Vec<String>,
    /// Inheritance conflict resolution direction (`left`, or `right`, default is `left`).
    pub inheritance_direction: Option<String>,
    /// Optional ProcedureBlock default for this class; If defined, methods will inherit this keyword if they don't specify it themselves.
    pub is_procedure_block: Option<bool>,
    /// Optional default Language keyword for this class.
    pub default_language: Option<Language>,
    /// Stores method name -> MethodRef for each method in this class.
    pub methods: HashMap<String, MethodRef>,
    /// Stores property name -> id for each property in this class.
    pub properties: HashMap<String, PropertyRef>,
    /// Stores parameter name -> id for each parameter in this class.
    pub parameters: HashMap<String, ParameterRef>,
    /// Whether this class entry is considered live/usable (e.g., false after removal).
    pub active: bool,
    /// Whether this representation is of a routine.
    pub is_rtn: bool,
    pub(crate) next_method_id: usize,
    pub(crate) next_parameter_id: usize,
    pub(crate) next_property_id: usize,
    /// If true, this class and all of its members cannot be overwritten by subclasses.
    pub is_final: Option<bool>,
}

/// Language keyword values supported for classes/methods.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Language {
    Objectscript,
    TSql,
    Python,
    ISpl,
}

/// Semantic representation of a class parameter declaration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClassParameter {
    pub name: String,
    pub property_type: Option<String>,
    pub default_argument_value: Option<String>, // this can be a numeric literal, string literal, or identifier
    pub range: Range,
}

/// Distinguishes instance methods from class methods.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum MethodType {
    InstanceMethod,
    ClassMethod,
    Procedure(bool),
    Subroutine(bool),
    DottedSubroutine(bool),
    Routine,
}

/// Reference linking a variable to its public and/or private identifier.
#[derive(Clone, Debug, Eq, PartialEq, Copy)]
pub struct VariableRef {
    pub pub_id: Option<PublicVarId>,
    pub priv_id: Option<PrivateVarId>,
}

/// Semantic Representation of an ObjectScript Method.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Method {
    /// Class Method or Instance Method.
    pub method_type: MethodType,
    /// Expected return type.
    pub return_type: Option<TypeName>,
    /// Method Name.
    pub name: String,
    /// Stores variable name -> VariableRef for all variable definitions in this method.
    pub variables: HashMap<String, Vec<(VariableRef, ScopeId)>>,
    /// Whether method is public or not.
    pub is_public: bool,
    /// Whether method is a procedure block or not. If None, method defaults to procedure block.
    pub is_procedure_block: Option<bool>,
    /// Stores language of method. If None, method defaults to ObjectScript.
    pub language: Option<Language>,
    /// Stores CodeMode of method. If None, method defaults to Code.
    pub code_mode: CodeMode,
    /// Names declared in `PublicList(...)` of ProcedureBlocks.
    pub public_variables_declared: HashSet<String>,
    /// If true, this method cannot be overwritten by subclasses.
    pub is_final: Option<bool>,
}

/// CodeMode keyword values supported for methods.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CodeMode {
    Call,
    Code,
    Expression,
    ObjectGenerator,
}

/// Parsed representation of a class method call expression (syntactic/semantic summary).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClassMethodCall {
    pub name: String,
    pub class_name: String,
    pub method_name: String,
    pub is_public: bool,
}

/// Semantic representation of a variable discovered in a method.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Variable {
    /// Variable name.
    pub name: String,
    /// Optional type of the argument if the variable originated from a method argument.
    pub arg_type: Option<TypeName>,
    /// Whether variable is public or not.
    pub is_public: bool,
    /// True if variable is an instance of a class, false otherwise.
    pub is_oref: bool,
    /// None if not an oref. If an oref, String representing class it points to.
    pub cls: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TypeName {
    pub ret_type: ReturnType,
    pub parameters: Vec<String>,
}

/// Normalized return/type categories recognized.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReturnType {
    String,
    Integer,
    TinyInteger, // has diff max and min values
    Number,
    Binary,
    Decimal,
    Boolean,
    Date,
    Status,
    TimeStamp,
    DynamicObject,
    DynamicArray,
    Float,
    Double,
    HttpResponse,
    Other(String),
    SqlQuery,
    ClassName,
    CosCode,
    CosIdentifier,
    SqlIdentifier,
    ConfigValue,
    Variable,
    Expression,
}

/// File type for a workspace document.
#[derive(Clone, Debug, Eq, PartialEq, Copy)]
pub enum FileType {
    Cls,
    Routine,
    Xml,
}

/// Parsed representation of a legacy statement targeted for refactoring.
#[derive(Clone, Debug)]
pub struct OldStatement {
    pub last_expression_end_byte: Option<usize>,
    pub last_expression_end_point: Option<tree_sitter::Point>,
    pub statement_ranges: Vec<std::ops::Range<usize>>,
    pub keyword_old_range: tree_sitter::Range,
    pub command_range: tree_sitter::Range,
    pub comment_range: Option<tree_sitter::Range>,
    pub comment_after_last_statement_range: Option<tree_sitter::Range>,
    pub statements_after: Vec<std::ops::Range<usize>>,
}

/// A routine block generated during refactoring to hold extracted code.
#[derive(Clone, Debug)]
pub struct GeneratedRoutineBlock {
    pub name: String,
    pub text: String,
    pub insert_at: usize,
}

/// A single refactoring operation pairing a text replacement with a generated routine block.
#[derive(Clone, Debug)]
pub struct RefactorStep {
    pub replace_range: std::ops::Range<usize>,
    pub replacement: String,
    pub generated_block: GeneratedRoutineBlock,
}

/// Controls which statement types are included in a refactoring pass.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RefactorLevel {
    All,
    DoCommands,
    Conditionals,
    ForCommands,
}

/// Categorizes a statement by its control-flow construct type.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StatementType {
    For,
    If,
    Conditionals,
    Else,
}
