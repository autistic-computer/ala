use ala::{Action, actions};
use ala_macros::register_action;
use schemars::JsonSchema;
use serde::Deserialize;

#[test]
fn test_action_macros() {
    actions!(test_only, [SomeAction, SomeActionWithDocs,]);

    #[derive(PartialEq, Clone, Deserialize, JsonSchema, Action)]
    #[action(namespace = test_only)]
    #[serde(deny_unknown_fields)]
    struct AnotherAction;

    #[derive(PartialEq, Clone, ala::private::serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct RegisterableAction {}

    register_action!(RegisterableAction);

    impl ala::Action for RegisterableAction {
        fn boxed_clone(&self) -> Box<dyn ala::Action> {
            unimplemented!()
        }

        fn partial_eq(&self, _action: &dyn ala::Action) -> bool {
            unimplemented!()
        }

        fn name(&self) -> &'static str {
            unimplemented!()
        }

        fn name_for_type() -> &'static str
        where
            Self: Sized,
        {
            unimplemented!()
        }

        fn build(_value: serde_json::Value) -> anyhow::Result<Box<dyn ala::Action>>
        where
            Self: Sized,
        {
            unimplemented!()
        }
    }
}
