//! Dependency Graph and Layered Topological Build Order Resolution using Kahn's Algorithm.
//!
//! Constructs a Directed Acyclic Graph (DAG) of packages based on `BuildRequires` and `Provides`,
//! and schedules parallel compilation layers using Kahn's topological sort (In-Degree BFS).
//!
//! Reference:
//! Kahn, A. B. (1962). "Topological sorting of large networks".
//! Communications of the ACM, 5(11), 558–562.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use eyre::{eyre, Result};

use crate::distgit::spec::{parse_spec_file, SpecMetadata};

/// Represents a distinct parallel compilation layer.
/// All packages within a layer have had all their workspace prerequisites satisfied
/// and can be safely compiled concurrently across parallel Mock workers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompilationLayer {
    /// Zero-based layer index (Layer 0 contains packages with no unbuilt workspace prerequisites).
    pub layer_index: usize,
    /// Package names scheduled for execution in this layer.
    pub packages: Vec<String>,
}

/// Directed package dependency graph.
#[derive(Clone, Debug, Default)]
pub struct DependencyGraph {
    /// Mapping of package name to its parsed spec metadata.
    pub packages: HashMap<String, SpecMetadata>,
    /// Capability provider map: provides string -> package name.
    pub provides_map: HashMap<String, String>,
    /// Adjacency list: package -> set of workspace packages it depends on (BuildRequires).
    pub dependencies: HashMap<String, HashSet<String>>,
    /// Reverse adjacency list: package -> set of workspace packages that depend on it.
    pub dependents: HashMap<String, HashSet<String>>,
}

impl DependencyGraph {
    /// Creates an empty dependency graph.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a package and its spec metadata to the graph.
    pub fn add_package(&mut self, meta: SpecMetadata) {
        let pkg_name = meta.name.clone();

        // The package provides itself by default
        self.provides_map.insert(pkg_name.clone(), pkg_name.clone());

        // Extract base package names from provides if versioned
        for prov in &meta.provides {
            let clean_prov = prov.split_whitespace().next().unwrap_or(prov).to_string();
            self.provides_map.insert(clean_prov, pkg_name.clone());
        }

        self.packages.insert(pkg_name.clone(), meta);
        self.dependencies.entry(pkg_name.clone()).or_default();
        self.dependents.entry(pkg_name).or_default();
    }

    /// Scans a directory for dist-git repositories and `.spec` files, parsing all packages.
    pub fn load_from_dir(&mut self, dir: &Path) -> Result<usize> {
        let entries = match fs::read_dir(dir) {
            Ok(e) => e,
            Err(e) => return Err(eyre!("Failed to read directory {}: {}", dir.display(), e)),
        };

        let mut loaded = 0;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Ok(sub_entries) = fs::read_dir(&path) {
                    for sub in sub_entries.flatten() {
                        let sub_path = sub.path();
                        if sub_path.extension().and_then(|ext| ext.to_str()) == Some("spec") {
                            match parse_spec_file(&sub_path) {
                                Ok(meta) => {
                                    self.add_package(meta);
                                    loaded += 1;
                                }
                                Err(e) => eprintln!("Warning: failed to parse {}: {}", sub_path.display(), e),
                            }
                        }
                    }
                }
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("spec") {
                match parse_spec_file(&path) {
                    Ok(meta) => {
                        self.add_package(meta);
                        loaded += 1;
                    }
                    Err(e) => eprintln!("Warning: failed to parse {}: {}", path.display(), e),
                }
            }
        }

        self.resolve_edges();
        Ok(loaded)
    }

    /// Resolves dependency edges between all loaded packages based on `BuildRequires`.
    pub fn resolve_edges(&mut self) {
        let pkg_names: Vec<String> = self.packages.keys().cloned().collect();

        for pkg in &pkg_names {
            let meta = match self.packages.get(pkg) {
                Some(m) => m,
                None => continue,
            };

            for req in &meta.build_requires {
                let clean_req = req.split_whitespace().next().unwrap_or(req).trim();

                // Check if this capability is provided by one of our workspace packages
                if let Some(provider) = self.provides_map.get(clean_req) {
                    if provider != pkg {
                        self.dependencies
                            .entry(pkg.clone())
                            .or_default()
                            .insert(provider.clone());

                        self.dependents
                            .entry(provider.clone())
                            .or_default()
                            .insert(pkg.clone());
                    }
                }
            }
        }
    }

    /// Computes parallel compilation layers using Kahn's algorithm (In-Degree BFS).
    ///
    /// Layer 0 contains all packages with zero unresolved workspace prerequisites.
    /// As each layer finishes, dependent packages have their in-degree decremented,
    /// naturally forming subsequent parallel compilation layers.
    ///
    /// If circular dependencies exist, returns an error identifying the cyclic packages.
    pub fn compute_layers(&self) -> Result<Vec<CompilationLayer>> {
        let mut in_degrees: HashMap<String, usize> = HashMap::new();
        let mut reverse_graph: HashMap<String, HashSet<String>> = HashMap::new();

        for pkg in self.packages.keys() {
            let prereqs = self.dependencies.get(pkg).cloned().unwrap_or_default();
            in_degrees.insert(pkg.clone(), prereqs.len());
            for p in prereqs {
                reverse_graph.entry(p).or_default().insert(pkg.clone());
            }
        }

        // Layer 0: packages with 0 prerequisites in the workspace
        let mut current_layer: Vec<String> = in_degrees
            .iter()
            .filter(|(_, deg)| **deg == 0)
            .map(|(k, _)| k.clone())
            .collect();
        current_layer.sort();

        let mut layers = Vec::new();
        let mut processed_count = 0;
        let mut layer_idx = 0;

        while !current_layer.is_empty() {
            processed_count += current_layer.len();
            let mut next_layer = Vec::new();

            for pkg in &current_layer {
                if let Some(dependents) = reverse_graph.get(pkg) {
                    for dep in dependents {
                        if let Some(deg) = in_degrees.get_mut(dep) {
                            *deg -= 1;
                            if *deg == 0 {
                                next_layer.push(dep.clone());
                            }
                        }
                    }
                }
            }

            layers.push(CompilationLayer {
                layer_index: layer_idx,
                packages: current_layer,
            });

            layer_idx += 1;
            next_layer.sort();
            current_layer = next_layer;
        }

        if processed_count < self.packages.len() {
            let mut unresolved: Vec<String> = in_degrees
                .into_iter()
                .filter(|(_, deg)| *deg > 0)
                .map(|(pkg, deg)| format!("{} (unresolved prerequisites: {})", pkg, deg))
                .collect();
            unresolved.sort();

            return Err(eyre!(
                "Circular dependency detected! Cannot schedule build order for: {}",
                unresolved.join(", ")
            ));
        }

        Ok(layers)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kahn_layers_linear() {
        let mut graph = DependencyGraph::new();

        let mut meta_a = SpecMetadata::default();
        meta_a.name = "pkg-a".to_string();

        let mut meta_b = SpecMetadata::default();
        meta_b.name = "pkg-b".to_string();
        meta_b.build_requires = vec!["pkg-a".to_string()];

        let mut meta_c = SpecMetadata::default();
        meta_c.name = "pkg-c".to_string();
        meta_c.build_requires = vec!["pkg-b".to_string()];

        graph.add_package(meta_a);
        graph.add_package(meta_b);
        graph.add_package(meta_c);
        graph.resolve_edges();

        let layers = graph.compute_layers().expect("Expected successful layer calculation");
        assert_eq!(layers.len(), 3);
        assert_eq!(layers[0].packages, vec!["pkg-a".to_string()]);
        assert_eq!(layers[1].packages, vec!["pkg-b".to_string()]);
        assert_eq!(layers[2].packages, vec!["pkg-c".to_string()]);
    }

    #[test]
    fn test_kahn_parallel_branches() {
        let mut graph = DependencyGraph::new();

        let mut meta_a = SpecMetadata::default();
        meta_a.name = "pkg-a".to_string();

        let mut meta_b = SpecMetadata::default();
        meta_b.name = "pkg-b".to_string();

        let mut meta_c = SpecMetadata::default();
        meta_c.name = "pkg-c".to_string();
        meta_c.build_requires = vec!["pkg-a".to_string(), "pkg-b".to_string()];

        graph.add_package(meta_a);
        graph.add_package(meta_b);
        graph.add_package(meta_c);
        graph.resolve_edges();

        let layers = graph.compute_layers().expect("Expected successful layer calculation");
        assert_eq!(layers.len(), 2);
        assert_eq!(layers[0].packages, vec!["pkg-a".to_string(), "pkg-b".to_string()]);
        assert_eq!(layers[1].packages, vec!["pkg-c".to_string()]);
    }

    #[test]
    fn test_kahn_cycle_detected() {
        let mut graph = DependencyGraph::new();

        let mut meta_x = SpecMetadata::default();
        meta_x.name = "pkg-x".to_string();
        meta_x.build_requires = vec!["pkg-y".to_string()];

        let mut meta_y = SpecMetadata::default();
        meta_y.name = "pkg-y".to_string();
        meta_y.build_requires = vec!["pkg-x".to_string()];

        graph.add_package(meta_x);
        graph.add_package(meta_y);
        graph.resolve_edges();

        let result = graph.compute_layers();
        assert!(result.is_err(), "Expected cycle detection error");
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("Circular dependency detected"));
        assert!(err_msg.contains("pkg-x"));
        assert!(err_msg.contains("pkg-y"));
    }
}
