use crate::tools::Tools;
use mcplease::traits::AsToolsList;

#[test]
fn schemars_dont_panic() {
    Tools::tools_list();
}
