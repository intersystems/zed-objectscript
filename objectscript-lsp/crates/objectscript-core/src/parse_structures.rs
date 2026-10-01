use crate::scope_structures::ScopeId;
use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::hash::Hasher;
use tower_lsp::lsp_types::Range as LspRange;
use tree_sitter::{Parser, Range};
use tree_sitter_objectscript::LANGUAGE_OBJECTSCRIPT_UDL;
use tree_sitter_objectscript_routine::LANGUAGE_OBJECTSCRIPT_ROUTINE;
use tree_sitter_xml::LANGUAGE_XML;
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

/// Stores the Relationship Index, which is assigned by `class.get_next_relationship_id()`.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct RelationshipId(pub usize);

/// Stores the ForeignKey Index, which is assigned by `class.get_next_foreign_key_id()`.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct ForeignKeyId(pub usize);

/// Stores the Query Index, which is assigned by `class.get_next_query_key_id()`.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct QueryId(pub usize);

/// Stores the Argument Index, which is assigned by `method.get_next_argument_id()`.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct ArgumentId(pub usize);

/// Stores the Index Index, which is assigned by `class.get_next_index_key_id()`.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct IndexId(pub usize);

/// Stores the ForeignKey Index, which is assigned by `class.get_next_trigger_key_id()`.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct TriggerId(pub usize);

/// Stores the Xdata Index, which is assigned by `class.get_next_xdata_key_id()`.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct XdataId(pub usize);

/// Stores the Projection Index, which is assigned by `class.get_next_projection_id()`.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct ProjectionId(pub usize);

/// Stores the storage Index, which is assigned by `class.get_next_storage_id()`.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct StorageId(pub usize);

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
    ClientMethod,
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
    InheritedClass,
    ClassKeyword,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrackedKeywords {
    pub(crate) is_final: Option<bool>,
    pub(crate) is_public: bool,
    pub(crate) is_required: bool,
    pub(crate) multidimensional: bool,
    pub(crate) inverse: Option<String>,
    pub(crate) cardinality: Option<Cardinality>,
    pub(crate) on_delete: ForeignKeyAction,
    pub(crate) on_update: ForeignKeyAction,
    pub(crate) requires: Vec<String>,
    pub(crate) index_type: IndexType,
    pub(crate) trigger_insert: bool,
    pub(crate) trigger_delete: bool,
    pub(crate) trigger_update: bool,
    pub(crate) trigger_time: TriggerFire,
    pub(crate) trigger_for_each: TriggerForEach,
    pub(crate) language: Option<Language>,
    pub(crate) code_mode: CodeMode,
    pub(crate) procedure_block: Option<bool>,
    pub(crate) public_variables_declared: HashSet<String>,
}

impl Default for TrackedKeywords {
    fn default() -> Self {
        Self {
            is_final: None,
            is_public: true,
            is_required: false,
            multidimensional: false,
            inverse: None,
            cardinality: None,
            on_delete: ForeignKeyAction::NoAction,
            on_update: ForeignKeyAction::NoAction,
            requires: Vec::new(),
            index_type: IndexType::Index,
            trigger_insert: false,
            trigger_update: false,
            trigger_delete: false,
            trigger_time: TriggerFire::BEFORE,
            trigger_for_each: TriggerForEach::Row,
            language: None,
            code_mode: CodeMode::Code,
            procedure_block: None,
            public_variables_declared: HashSet::new(),
        }
    }
}

pub struct IndexParsers {
    pub cls: Parser,
    pub routine: Parser,
    pub xml: Parser,
}

impl IndexParsers {
    pub fn new() -> Self {
        let mut cls = Parser::new();
        cls.set_language(&LANGUAGE_OBJECTSCRIPT_UDL.into())
            .expect("failed to load ObjectScript UDL grammar");

        let mut routine = Parser::new();
        routine
            .set_language(&LANGUAGE_OBJECTSCRIPT_ROUTINE.into())
            .expect("failed to load ObjectScript routine grammar");

        let mut xml = Parser::new();
        xml.set_language(&LANGUAGE_XML.into())
            .expect("failed to load XML grammar");

        Self { cls, routine, xml }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum InheritanceDirection {
    Left,
    Right,
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

/// Reference to a relationship in a class.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct RelationshipRef {
    pub class: ClassId,
    pub id: RelationshipId,
}

/// Reference to a ForeignKey in a class.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct ForeignKeyRef {
    pub class: ClassId,
    pub id: ForeignKeyId,
}

/// Reference to a Index in a class.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct IndexRef {
    pub class: ClassId,
    pub id: IndexId,
}

/// Reference to a trigger in a class.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct TriggerRef {
    pub class: ClassId,
    pub id: TriggerId,
}

/// Reference to a xdata in a class.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct XdataRef {
    pub class: ClassId,
    pub id: XdataId,
}

/// Reference to a query in a class.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct QueryRef {
    pub class: ClassId,
    pub id: QueryId,
}

/// Reference to a projection in a class.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct ProjectionRef {
    pub class: ClassId,
    pub id: ProjectionId,
}

/// Reference to a storage in a class.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct StorageRef {
    pub class: ClassId,
    pub id: StorageId,
}

/// Reference to an argument in a method.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct ArgumentRef {
    pub method: MethodRef,
    pub id: ArgumentId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Parameter {
    /// If true, the Parameter cannot be overwritten by subclasses.
    pub is_final: Option<bool>,
    /// Parameter Name.
    pub name: String,
    /// Expected return type.
    pub return_type: Option<TypeName>,
    /// Optional Default Value.
    pub default_value: Option<String>,
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
    /// Argument name -> Argument, Range
    pub arguments: HashMap<String, (Argument, Range)>,
    /// The next Id available for a new argument.
    pub next_argument_id: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ForeignKey {
    /// ForeignKey Name.
    pub name: String,
    /// names of properties constrained by this key
    pub properties_constrained: Vec<String>,
    /// name of class referenced
    pub referenced_class: String,
    /// name of index within referenced class
    pub referenced_index: Option<String>,
    /// Specifies action that this foreign key should cause in the table when the key value of a record in the table is updated.
    pub on_update: ForeignKeyAction,
    /// Specifies action that this foreign key should cause in the table when the key value of a record in the table is deleted.
    pub on_delete: ForeignKeyAction,
}

#[derive(Clone, Debug, Eq, PartialEq, Copy)]
pub enum ForeignKeyAction {
    NoAction,
    SetDefault,
    SetNull,
    Cascade,
}

#[derive(Clone, Debug, Eq, PartialEq, Copy)]
pub enum Cardinality {
    Children,
    Parent,
    Many,
    One,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Relationship {
    /// Whether Relationship is required or not.
    pub required: bool,
    /// Whether Relationship is final or not.
    pub is_final: Option<bool>,
    /// Whether Relationship is public or not.
    pub is_public: bool,
    /// Relationship Name.
    pub name: String,
    /// Specifies the cardinality.
    pub cardinality: Cardinality,
    /// Expected return type.
    pub return_type: Option<TypeName>,
    /// Specifies the inverse side of this relationship.
    pub inverse: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Query {
    /// Specifies a list of privileges a user or process must have to call this query.
    pub required_privileges: Vec<String>,
    /// Whether Query is final or not.
    pub is_final: Option<bool>,
    /// Whether Query is public or not.
    pub is_public: bool,
    /// Query Name.
    pub name: String,
    /// Specifies the query class used by this query.
    pub return_type: TypeName,
    /// Query argument declarations keyed by argument name.
    pub arguments: HashMap<String, (Argument, Range)>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Projection {
    /// Whether Projection is final or not.
    pub is_final: Option<bool>,
    /// Projection Name.
    pub name: String,
    /// Specifies the query class used by this query.
    pub return_type: TypeName,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Index {
    /// Index Name.
    pub name: String,
    /// Properties that the index is based on
    pub properties: Vec<IndexPropertyValue>,
    /// Specifies the Index Type, default is Index.
    pub index_type: IndexType,
    /// Specifies expected return type.
    pub return_type: Option<TypeName>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Trigger {
    /// Trigger Name.
    pub name: String,
    /// Stores CodeMode of Trigger. If None, Trigger defaults to Code.
    pub code_mode: CodeMode,
    /// If true, this trigger cannot be inherited by subclasses.
    pub is_final: Option<bool>,
    /// Stores language of trigger, defaults to ObjectScript.
    pub language: Language,
    /// if true, this trigger is fired during an SQL DELETE operation.
    pub delete: bool,
    /// if true, this trigger is fired during an SQL UPDATE operation.
    pub update: bool,
    /// if true, this trigger is fired during an SQL INSERT operation.
    pub insert: bool,
    /// Specifies whether Trigger Fires Before or After Event. Default is BEFORE.
    pub time: TriggerFire,
    /// Specifies when Trigger is Fired. Default is row.
    pub for_each: TriggerForEach,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct XData {
    /// XData Block Name.
    pub name: String,
    /// Specifies the Language of the Xdata Block. Default is Xml.
    pub language: Language,
}

/// Specifies whether Trigger Fires Before or After Event.
#[derive(Clone, Debug, Eq, PartialEq, Copy)]
pub enum TriggerFire {
    BEFORE,
    AFTER,
}

#[derive(Clone, Debug, Eq, PartialEq, Copy)]
pub enum TriggerForEach {
    Row,       // This trigger is fired by each row affected by the triggering statement.
    RowObject, // This trigger is fired by each row affected by the triggering statement or by changes via object access.
    Statement, // This trigger is fired once for the whole statement.
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IndexPropertyValue {
    pub name: String,
    pub elements: bool,
    pub keys: bool,
    pub return_type: Option<TypeName>,
}

#[derive(Clone, Debug, Eq, PartialEq, Copy)]
pub enum IndexType {
    CollatedKey,
    Bitslice,
    Columnar,
    Bitmap,
    Index,
    Key,
    Extent,
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
    pub inherited_classes: Vec<(String, LspRange)>,
    /// Inheritance conflict resolution direction (`left`, or `right`, default is `left`).
    pub inheritance_direction: InheritanceDirection,
    /// Optional ProcedureBlock default for this class; If defined, methods will inherit this keyword if they don't specify it themselves.
    pub is_procedure_block: bool,
    /// Optional default Language keyword for this class.
    pub default_language: Language,
    /// Stores method name -> MethodRef for each method in this class.
    pub methods: HashMap<String, MethodRef>,
    /// Stores property name -> id for each property in this class.
    pub properties: HashMap<String, PropertyRef>,
    /// Stores parameter name -> id for each parameter in this class.
    pub parameters: HashMap<String, ParameterRef>,
    /// Stores relationship name -> id for each relationship in this class.
    pub relationships: HashMap<String, RelationshipRef>,
    /// Stores ForeignKey name -> id for each ForeignKey in this class.
    pub foreignkeys: HashMap<String, ForeignKeyRef>,
    /// Stores query name -> id for each query in this class.
    pub queries: HashMap<String, QueryRef>,
    /// Stores Index name -> id for each Index in this class.
    pub indices: HashMap<String, IndexRef>,
    /// Stores trigger name -> id for each trigger in this class.
    pub triggers: HashMap<String, TriggerRef>,
    /// Stores projection name -> id for each Projection in this class.
    pub projections: HashMap<String, ProjectionRef>,
    /// Stores Xdata name -> XdataRef for each Xdata member in this class.
    pub xdata: HashMap<String, XdataRef>,
    /// Stores storage name -> StorageRef for each storage member in this class.
    pub storage: HashMap<String, StorageRef>,
    /// Whether this class entry is considered live/usable (e.g., false after removal).
    pub active: bool,
    /// Whether this representation is of a routine.
    pub is_rtn: bool,
    pub(crate) next_method_id: usize,
    pub(crate) next_parameter_id: usize,
    pub(crate) next_property_id: usize,
    pub(crate) next_relationship_id: usize,
    pub(crate) next_index_id: usize,
    pub(crate) next_foreign_key_id: usize,
    pub(crate) next_query_id: usize,
    pub(crate) next_trigger_id: usize,
    pub(crate) next_xdata_id: usize,
    pub(crate) next_projection_id: usize,
    pub(crate) next_storage_id: usize,
    /// If true, this class and all of its members cannot be inherited by subclasses.
    pub is_final: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Storage {
    /// Storage Name
    pub name: String,
}

/// Language keyword values supported for classes/methods.
#[derive(Clone, Debug, Eq, PartialEq, Copy)]
pub enum Language {
    Objectscript,
    TSql,
    ISpl,
    Basic,
    Json,
    Html,
    JavaScript,
    Css,
    Sql,
    Java,
    Python,
    Xml,
    Yaml,
    Markdown,
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
    ClientMethod,
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
    /// Stores argument name -> Argument for all arguments in this method.
    pub arguments: HashMap<String, (Argument, Range)>,
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
    /// Tracks the next available id for an argument in the method.
    pub next_argument_id: usize,
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Argument {
    /// Argument name.
    pub name: String,
    /// Return Type
    pub return_type: Option<TypeName>,
    /// Default Value for Arg
    pub default_value: Option<String>,
    /// Indicate that an argument should be passed by reference and is intended to have no incoming value.
    pub output: bool,
    /// If true, the method modifies the value of the variable outside the method.
    pub byref: bool,
}

impl Default for Argument {
    fn default() -> Self {
        Self {
            name: "TODO".to_string(),
            return_type: None,
            default_value: None,
            output: false,
            byref: false,
        }
    }
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
    /// The Type of Variable Definition.
    pub variable_type: VariableDefType,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OrefChainExpr {
    pub class_ref: String,
    pub property_ref: Option<String>,
    pub parameter_ref: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VariableDefType {
    OrefDef(String),
    OrefChainExpr(OrefChainExpr),
    VariableDef,
    PropertyDef((String, String)),
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
