pub mod advanced_rules;
pub mod bare_import_in_list;
pub mod builtin_argument_type;
pub mod builtin_value_rules;
pub mod call_simplification;
pub mod cognitive_complexity;
pub mod config_file_rules;
mod config_text_generator;
pub mod constant_option;
pub mod deprecated_package_alias;
pub mod duplicate_attr;
pub mod duplicate_definitions;
pub mod duplicate_formal;
pub mod duplicate_inherit;
pub mod duplicate_list_to_attrs_name;
pub mod file_rules;
pub mod flake_inputs;
pub mod ignored_function_argument;
pub mod ill_typed_binop;
pub mod ill_typed_unary_op;
pub mod interpolation_rules;
pub mod invalid_list_access;
mod lib_helpers;
pub mod lib_type_rules;
pub mod lua_data_rules;
pub mod machine_local_default;
pub mod missing_attribute;
pub mod missing_function_argument;
pub mod module_rules;
pub mod never_enabled_option;
pub mod node_rules;
pub mod non_boolean_condition;
pub mod non_callable_application;
pub mod path_rules;
pub mod per_system_input_lookup;
pub mod reference_rules;
pub mod repeated_dynamic_path;
pub mod repeated_let_value;
pub mod scattered_documentation;
pub mod schema;
pub mod shallow_merge_overwrite;
pub mod simplification_rules;
pub mod singleton_attrset;
pub mod singleton_list;
pub mod singleton_optionals;
mod static_binding;
pub mod style_rules;
pub mod suggested_rules;
pub mod templated_bindings;
pub mod unexpected_function_argument;
pub mod unformatted_file;
pub mod unused_option;
pub mod wrapped_prose;

use advanced_rules::{
    BuiltinArity, DuplicateFunctionArgument, DynamicImport, ImportFromDerivation,
    SuspiciousImportArgument, SuspiciousRecursion, UnreachableBranch, UnsafeWithShadowing,
};
use bare_import_in_list::BareImportInList;
use builtin_argument_type::BuiltinArgumentType;
use builtin_value_rules::{
    InvalidBuiltinRange, InvalidListToAttrsEntry, ReplaceStringsLengthMismatch,
};
use call_simplification::{
    DeprecatedIsNull, ManualFileContents, ManualGenAttrs, ManualGetattr, ManualHasattr,
    ManualOptional, OptionalListArgument,
};
use cognitive_complexity::CognitiveComplexity;
use config_file_rules::{HandMaintainedDataFile, HandWrittenConfig, MaskedGenerator};
use constant_option::ConstantOption;
use deprecated_package_alias::DeprecatedPackageAlias;
use duplicate_attr::DuplicateAttribute;
use duplicate_definitions::DuplicateDefinitions;
use duplicate_formal::DuplicateFormal;
use duplicate_inherit::DuplicateInherit;
use duplicate_list_to_attrs_name::DuplicateListToAttrsName;
use file_rules::{
    CircularLet, CyclomaticComplexity, ImportCycle, MissingImport, ReboundConstant, RedundantWith,
    SelfReferentialLet, ShadowedBinding, ShadowedFormal, UnnecessaryOr, UnusedFormal,
    UnusedInherit, UnusedLambdaParam, UnusedLetBinding,
};
use flake_inputs::{DeadFlakeInput, FlakeInputWithoutNixpkgsFollows};
use ignored_function_argument::IgnoredFunctionArgument;
use ill_typed_binop::IllTypedBinop;
use ill_typed_unary_op::IllTypedUnaryOp;
use interpolation_rules::{CoercedInterpolation, RedundantInterpolation, RedundantQuotedAttr};
use invalid_list_access::InvalidListAccess;
use lib_type_rules::{DiscouragedLibType, UnknownLibType};
use lua_data_rules::HandWrittenLuaData;
use machine_local_default::MachineLocalDefault;
use missing_attribute::MissingAttribute;
use missing_function_argument::MissingFunctionArgument;
use module_rules::{ConfigDependentImports, MixedModuleSyntax, RedundantPriority};
use never_enabled_option::NeverEnabledOption;
use node_rules::{AssertTrue, ConstantIf, ConstantIfBranches, Tautology};
use non_boolean_condition::NonBooleanCondition;
use non_callable_application::NonCallableApplication;
use path_rules::{AccidentalPathDivision, DanglingPath, SearchPathReference, UnfilteredSrc};
use per_system_input_lookup::PerSystemInputLookup;
use reference_rules::{UndefinedVariable, UnknownBuiltin};
use repeated_dynamic_path::RepeatedDynamicPath;
use repeated_let_value::RepeatedLetValue;
use scattered_documentation::ScatteredDocumentation;
use schema::{OptionTypeMismatch, UnknownOption};
use shallow_merge_overwrite::ShallowMergeOverwrite;
use simplification_rules::{
    BooleanIf, ConstantBooleanBinop, ConstantBooleanNot, EmptyAttrsetMerge, IdentityLambda,
    NegationSimplification, TrivialLet,
};
use singleton_attrset::SingletonAttrset;
use singleton_list::SingletonList;
use singleton_optionals::SingletonOptionals;
use strictix_core::rules::Rule;
use style_rules::{
    CollapsibleLetIn, DeprecatedToPath, EmptyInherit, EmptyLetIn, EmptyListConcat, EmptyPattern,
    EtaReduction, ManualInherit, ManualInheritFrom, RedundantPatternBind, RepeatedKeys,
    SingletonListConcat, UnquotedUri, UselessHasAttr, UselessParens,
};
use suggested_rules::{
    AssertFalse, DuplicateLiteralListItem, LiteralDivisionByZero, RedundantBooleanComparison,
    UnnecessaryRec, UnusedRecBinding,
};
use templated_bindings::TemplatedBindings;
use unexpected_function_argument::UnexpectedFunctionArgument;
use unformatted_file::UnformattedFile;
use unused_option::UnusedOption;
use wrapped_prose::WrappedProse;

pub(crate) fn safe_delete_replacement(
    source: &str,
    range: strictix_syntax::TextRange,
) -> &'static str {
    let before = source
        .as_bytes()
        .get(range.start().saturating_sub(1) as usize);
    let after = source.as_bytes().get(range.end() as usize);
    let is_word = |b: u8| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'\'' | b'-');
    match (before, after) {
        (Some(&b), Some(&a)) if range.start() > 0 && is_word(b) && is_word(a) => " ",
        _ => "",
    }
}

#[must_use]
pub fn all_rules() -> Vec<Box<dyn Rule>> {
    strictix_core::rules! {
        MissingAttribute,
        DuplicateListToAttrsName,
        ShallowMergeOverwrite,
        BuiltinArgumentType,
        InvalidListToAttrsEntry,
        ReplaceStringsLengthMismatch,
        InvalidBuiltinRange,
        MixedModuleSyntax,
        ConfigDependentImports,
        InvalidListAccess,
        MissingFunctionArgument,
        UnexpectedFunctionArgument,
        IgnoredFunctionArgument,
        BuiltinArity,
        DuplicateFunctionArgument,
        DynamicImport,
        SuspiciousImportArgument,
        SuspiciousRecursion,
        UnreachableBranch,
        UnsafeWithShadowing,
        ConstantIf,
        ConstantIfBranches,
        AssertTrue,
        Tautology,
        UnusedLetBinding,
        UnusedLambdaParam,
        UnusedFormal,
        UnusedInherit,
        ShadowedBinding,
        ShadowedFormal,
        UnnecessaryOr,
        RedundantWith,
        SelfReferentialLet,
        UndefinedVariable,
        UnknownBuiltin,
        UnknownOption,
        UnusedOption,
        NeverEnabledOption,
        ConstantOption,
        DiscouragedLibType,
        ManualGenAttrs,
        ManualFileContents,
        EmptyLetIn,
        ManualInherit,
        ManualInheritFrom,
        CollapsibleLetIn,
        EtaReduction,
        EmptyPattern,
        RedundantPatternBind,
        EmptyInherit,
        DeprecatedToPath,
        UselessHasAttr,
        EmptyListConcat,
        SingletonListConcat,
        UselessParens,
        RepeatedKeys,
        UnquotedUri,
        DuplicateAttribute,
        DuplicateFormal,
        DuplicateInherit,
        NonBooleanCondition,
        IllTypedBinop,
        IllTypedUnaryOp,
        NonCallableApplication,
        MissingImport,
        ImportCycle,
        BareImportInList,
        UnnecessaryRec,
        UnusedRecBinding,
        AssertFalse,
        LiteralDivisionByZero,
        RedundantBooleanComparison,
        DuplicateLiteralListItem,
        DuplicateDefinitions,
        RepeatedDynamicPath,
        RepeatedLetValue,
        PerSystemInputLookup,
        CircularLet,
        CyclomaticComplexity,
        CognitiveComplexity,
        ReboundConstant,
        CoercedInterpolation,
        RedundantInterpolation,
        RedundantQuotedAttr,
        DanglingPath,
        AccidentalPathDivision,
        SearchPathReference,
        OptionTypeMismatch,
        UnknownLibType,
        BooleanIf,
        ConstantBooleanNot,
        ConstantBooleanBinop,
        IdentityLambda,
        EmptyAttrsetMerge,
        NegationSimplification,
        TrivialLet,
        ManualHasattr,
        ManualGetattr,
        DeprecatedIsNull,
        DeprecatedPackageAlias,
        ManualOptional,
        OptionalListArgument,
        SingletonOptionals,
        SingletonAttrset,
        SingletonList,
        TemplatedBindings,
        MaskedGenerator,
        HandWrittenConfig,
        HandMaintainedDataFile,
        HandWrittenLuaData,
        ScatteredDocumentation,
        UnformattedFile,
        FlakeInputWithoutNixpkgsFollows,
        DeadFlakeInput,
        RedundantPriority,
        ImportFromDerivation,
        UnfilteredSrc,
        MachineLocalDefault,
        WrappedProse,
    }
}
