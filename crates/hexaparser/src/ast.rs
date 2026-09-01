use std::collections::BTreeMap;

/// An expression in the compiler's intentionally small, structural AST.
#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Binary(BinaryExpr),
    In(InExpr),
    Ternary(TernaryExpr),
    Unary(UnaryExpr),
    StringCast(StringCastExpr),
    Cast(CastExpr),
    Member(MemberExpr),
    DynamicMember(DynamicMemberExpr),
    DynamicVar(DynamicVarExpr),
    ArrayIndex(ArrayIndexExpr),
    MultiArrayIndex(MultiArrayIndexExpr),
    Identifier(IdentifierExpr),
    Enum(EnumExpr),
    Number(NumberExpr),
    String(StringExpr),
    Bool(BoolExpr),
    Null,
    Call(CallExpr),
    ChainedCall(ChainedCallExpr),
    MethodCall(MethodCallExpr),
    DynamicMethodCall(DynamicMethodCallExpr),
    NewObject(NewObjectExpr),
    NewArray(NewArrayExpr),
    Lambda(LambdaExpr),
    ArrayLiteral(ArrayLiteralExpr),
}

#[derive(Clone, Debug, PartialEq)]
pub struct BinaryExpr { pub left: Box<Expr>, pub op: String, pub right: Box<Expr> }
#[derive(Clone, Debug, PartialEq)]
pub struct InExpr { pub expression: Box<Expr>, pub lower: Box<Expr>, pub upper: Option<Box<Expr>> }
#[derive(Clone, Debug, PartialEq)]
pub struct TernaryExpr { pub condition: Box<Expr>, pub when_true: Box<Expr>, pub when_false: Box<Expr> }
#[derive(Clone, Debug, PartialEq)]
pub struct UnaryExpr { pub op: String, pub expression: Box<Expr>, pub postfix: bool }
#[derive(Clone, Debug, PartialEq)]
pub struct StringCastExpr { pub expression: Box<Expr> }
#[derive(Clone, Debug, PartialEq)]
pub struct CastExpr { pub type_name: String, pub expression: Box<Expr> }
#[derive(Clone, Debug, PartialEq)]
pub struct MemberExpr { pub object: Box<Expr>, pub name: String }
#[derive(Clone, Debug, PartialEq)]
pub struct DynamicMemberExpr { pub object: Box<Expr>, pub name: Box<Expr> }
#[derive(Clone, Debug, PartialEq)]
pub struct DynamicVarExpr { pub name: Box<Expr> }
#[derive(Clone, Debug, PartialEq)]
pub struct ArrayIndexExpr { pub target: Box<Expr>, pub index: Box<Expr> }
#[derive(Clone, Debug, PartialEq)]
pub struct MultiArrayIndexExpr { pub target: Box<Expr>, pub indices: Vec<Expr> }
#[derive(Clone, Debug, PartialEq)]
pub struct IdentifierExpr { pub name: String }
#[derive(Clone, Debug, PartialEq)]
pub struct EnumExpr { pub enum_name: String, pub member_name: String }
#[derive(Clone, Debug, PartialEq)]
pub struct NumberExpr { pub text: String }
#[derive(Clone, Debug, PartialEq)]
pub struct StringExpr { pub value: String }
#[derive(Clone, Debug, PartialEq)]
pub struct BoolExpr { pub value: bool }
#[derive(Clone, Debug, PartialEq)]
pub struct CallExpr { pub name: String, pub args: Vec<Expr> }
#[derive(Clone, Debug, PartialEq)]
pub struct ChainedCallExpr { pub call: Box<CallExpr>, pub args: Vec<Expr> }
#[derive(Clone, Debug, PartialEq)]
pub struct MethodCallExpr { pub object: Box<Expr>, pub name: String, pub args: Vec<Expr> }
#[derive(Clone, Debug, PartialEq)]
pub struct DynamicMethodCallExpr { pub object: Box<Expr>, pub name: Box<Expr>, pub args: Vec<Expr> }
#[derive(Clone, Debug, PartialEq)]
pub struct NewObjectExpr { pub type_name: String, pub args: Vec<Expr> }
#[derive(Clone, Debug, PartialEq)]
pub struct NewArrayExpr { pub dimensions: Vec<Expr> }
#[derive(Clone, Debug, PartialEq)]
pub struct LambdaExpr { pub name: String, pub args: Vec<Expr>, pub body: Vec<Stmt> }
#[derive(Clone, Debug, PartialEq)]
pub struct ArrayLiteralExpr { pub values: Vec<Expr> }

#[derive(Clone, Debug, PartialEq)]
pub enum Stmt {
    Expr(ExprStmt),
    Inline(InlineStmt),
    Block(BlockStmt),
    Return(ReturnStmt),
    If(IfStmt),
    For(ForStmt),
    ForEach(ForEachStmt),
    While(WhileStmt),
    DoWhile(DoWhileStmt),
    With(WithStmt),
    Switch(SwitchStmt),
    New(NewStmt),
    Break,
    Continue,
    Goto(GotoStmt),
    Label(LabelStmt),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExprStmt { pub expression: Expr }
#[derive(Clone, Debug, PartialEq)]
pub struct InlineStmt { pub statement: Box<Stmt> }
#[derive(Clone, Debug, PartialEq)]
pub struct BlockStmt { pub body: Vec<Stmt> }
#[derive(Clone, Debug, PartialEq)]
pub struct ReturnStmt { pub expression: Expr }
#[derive(Clone, Debug, PartialEq)]
pub struct IfStmt { pub condition: Expr, pub then_body: Vec<Stmt>, pub else_body: Vec<Stmt>, pub has_else: bool }
#[derive(Clone, Debug, PartialEq)]
pub struct ForStmt { pub init: Option<Expr>, pub condition: Expr, pub post: Option<Expr>, pub body: Vec<Stmt> }
#[derive(Clone, Debug, PartialEq)]
pub struct ForEachStmt { pub name: Expr, pub source: Expr, pub body: Vec<Stmt> }
#[derive(Clone, Debug, PartialEq)]
pub struct WhileStmt { pub condition: Expr, pub body: Vec<Stmt> }
#[derive(Clone, Debug, PartialEq)]
pub struct DoWhileStmt { pub body: Vec<Stmt>, pub condition: Expr }
#[derive(Clone, Debug, PartialEq)]
pub struct WithStmt { pub target: Expr, pub body: Vec<Stmt> }
#[derive(Clone, Debug, PartialEq)]
pub struct SwitchStmt { pub expression: Expr, pub cases: Vec<SwitchCase> }
#[derive(Clone, Debug, PartialEq)]
pub struct NewStmt { pub type_name: String, pub args: Vec<Expr>, pub body: Vec<Stmt> }
#[derive(Clone, Debug, PartialEq)]
pub struct GotoStmt { pub label: String }
#[derive(Clone, Debug, PartialEq)]
pub struct LabelStmt { pub label: String }
#[derive(Clone, Debug, PartialEq)]
pub struct SwitchCase { pub labels: Vec<Option<Expr>>, pub body: Vec<Stmt> }

#[derive(Clone, Debug, PartialEq)]
pub struct ProgramNode {
    pub constants: BTreeMap<String, Expr>,
    pub enums: BTreeMap<String, BTreeMap<String, i32>>,
    pub items: Vec<ProgramItem>,
    pub gs1_event_flags: i32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ProgramItem { Function(FunctionNode), Statement(Stmt) }

#[derive(Clone, Debug, PartialEq)]
pub struct FunctionNode { pub name: String, pub object_name: String, pub public: bool, pub args: Vec<Expr>, pub body: Vec<Stmt> }

impl Expr {
    pub fn binary(left: Expr, op: impl Into<String>, right: Expr) -> Self {
        Self::Binary(BinaryExpr { left: Box::new(left), op: op.into(), right: Box::new(right) })
    }
    pub fn identifier(name: impl Into<String>) -> Self { Self::Identifier(IdentifierExpr { name: name.into() }) }
    pub fn number(text: impl Into<String>) -> Self { Self::Number(NumberExpr { text: text.into() }) }
    pub fn string(value: impl Into<String>) -> Self { Self::String(StringExpr { value: value.into() }) }
    pub fn boolean(value: bool) -> Self { Self::Bool(BoolExpr { value }) }
}

