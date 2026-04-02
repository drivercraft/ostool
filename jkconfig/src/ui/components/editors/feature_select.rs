use cursive::{Cursive, views::Dialog};
use std::{collections::HashMap, path::Path};

use crate::{
    data::{AppState, item::ItemType, types::ElementType},
    ui::{
        components::editors::multi_select_editor::{
            DepItem, ExtendedMultiSelectItem, show_extended_multi_select,
        },
        handle_back,
    },
};

/// 显示特性选择对话框，支持主要特性和依赖项特性的选择
///
/// # 参数
/// - `s`: Cursive UI 实例
/// - `package`: 要选择特性的包名
/// - `manifest_path`: Cargo.toml 文件路径
/// - `deps_filter`: 可选的依赖项过滤器
///   - `Some(&[String])`: 只显示指定的依赖项
///   - `None`: 显示所有有 features 的依赖项
///
/// # 功能
/// - 显示当前包的所有 features
/// - 显示依赖项的 features（可过滤）
/// - 支持多选操作
/// - 依赖项特性以 "dep_name/feature" 格式保存
pub fn show_feature_select(
    s: &mut Cursive,
    package: &str,
    manifest_path: &Path,
    deps_filter: Option<&[String]>,
) {
    if let Ok(metadata) = cargo_metadata::MetadataCommand::new()
        .manifest_path(manifest_path)
        .no_deps()
        .exec()
    {
        match metadata.packages.iter().find(|p| p.name == package) {
            Some(pkg) => {
                let all_features: Vec<String> = pkg.features.keys().cloned().collect();

                // 获取当前选中的特性列表
                let selected_values = if let Some(app) = s.user_data::<AppState>() {
                    if let Some(ElementType::Item(item)) = app.current() {
                        if let ItemType::Array(array) = &item.item_type {
                            array.values.clone()
                        } else {
                            Vec::new()
                        }
                    } else {
                        Vec::new()
                    }
                } else {
                    Vec::new()
                };

                // 分离主要特性和依赖项特性
                let mut main_features = Vec::new();
                let mut dep_selected_features = HashMap::new();

                for value in &selected_values {
                    if value.contains('/') {
                        // 这是依赖项特性，格式为 "dep_name/feature"
                        if let Some((dep_name, feature)) = value.split_once('/') {
                            dep_selected_features
                                .entry(dep_name.to_string())
                                .or_insert_with(Vec::new)
                                .push(feature.to_string());
                        }
                    } else {
                        // 这是主要特性
                        main_features.push(value.clone());
                    }
                }

                // 找到主要特性在所有特性中的索引
                let main_selected_indices: Vec<usize> = main_features
                    .iter()
                    .filter_map(|value| all_features.iter().position(|f| f == value))
                    .collect();

                // 获取依赖项信息
                let dependencies = get_package_dependencies(&metadata, package, deps_filter);

                // 将已选择的依赖项特性转换为索引
                let mut dep_selected_indices = HashMap::new();
                for dep in &dependencies {
                    if let Some(selected_features) = dep_selected_features.get(&dep.name) {
                        let indices: Vec<usize> = selected_features
                            .iter()
                            .filter_map(|feature| dep.features.iter().position(|f| f == feature))
                            .collect();
                        if !indices.is_empty() {
                            dep_selected_indices.insert(dep.name.clone(), indices);
                        }
                    }
                }

                // 创建ExtendedMultiSelectItem用于显示扩展多选界面
                let extended_multi_select_item = ExtendedMultiSelectItem {
                    variants: all_features,
                    selected_indices: main_selected_indices,
                    dependencies,
                    dep_selected_features: dep_selected_indices,
                };

                // 显示扩展多选对话框
                show_extended_multi_select(
                    s,
                    &format!("Features for {}", package),
                    &extended_multi_select_item,
                );
            }
            None => {
                let mut dialog =
                    Dialog::info(format!("Package '{}' not found in Cargo.toml", package));
                dialog
                    .buttons_mut()
                    .next()
                    .unwrap()
                    .set_callback(handle_back);

                s.add_layer(dialog);
            }
        }
    }
}

/// 获取包的依赖项信息，支持过滤
///
/// # 参数
/// - `metadata`: Cargo 元数据
/// - `package_name`: 要查询的包名
/// - `deps_filter`: 依赖项过滤器
///   - `Some(&[String])`: 只返回指定的依赖项
///   - `None`: 返回所有有 features 的依赖项
///
/// # 返回
/// 返回 `DepItem` 列表，每个包含依赖项名和其 features
/// 只包含有 features 的依赖项，按名称排序
fn get_package_dependencies(
    metadata: &cargo_metadata::Metadata,
    package_name: &str,
    deps_filter: Option<&[String]>,
) -> Vec<DepItem> {
    let mut dependencies = Vec::new();

    // 查找当前包
    if let Some(current_pkg) = metadata.packages.iter().find(|p| p.name == package_name) {
        // 遍历当前包的依赖项
        for dep in &current_pkg.dependencies {
            // 检查是否应该包含这个依赖项
            let should_include = match deps_filter {
                Some(filter_list) => filter_list.contains(&dep.name),
                None => true, // 如果没有过滤器，包含所有依赖项
            };

            if should_include {
                // 查找依赖项包的详细信息
                if let Some(dep_pkg) = metadata.packages.iter().find(|p| p.name == dep.name) {
                    let dep_features: Vec<String> = dep_pkg.features.keys().cloned().collect();

                    // 只添加有features的依赖项
                    if !dep_features.is_empty() {
                        dependencies.push(DepItem {
                            name: dep.name.clone(),
                            features: dep_features,
                        });
                    }
                }
            }
        }
    }

    // 按名称排序
    dependencies.sort_by(|a, b| a.name.cmp(&b.name));
    dependencies
}
