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
use std::path::{Path, PathBuf};
use eyre::{eyre, Result};
use log::{debug, info};

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

/// Represents a directed dependency edge that was severed to break a circular dependency.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrokenCycleEdge {
    /// Package that required the prerequisite.
    pub consumer: String,
    /// Prerequisite that was temporarily removed from workspace dependencies.
    pub prerequisite: String,
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
            debug!("Checking {:?}", entry);
            let path = entry.path();
            if path.is_dir() {
                if let Ok(sub_entries) = fs::read_dir(&path) {
                    for sub in sub_entries.flatten() {
                        let sub_path = sub.path();
                        if sub_path.extension().and_then(|ext| ext.to_str()) == Some("spec") {
                            debug!("Checking spec: {:?}", sub_path);
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
            // Entry is a spec file
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

    /// Loads specific package `.spec` files into the dependency graph and resolves their edges.
    pub fn load_package_targets(&mut self, spec_paths: &[PathBuf]) -> Result<usize> {
        let mut loaded = 0;
        for path in spec_paths {
            if path.is_file() {
                match parse_spec_file(path) {
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
                if let Some(provider) = self.provides_map.get(clean_req)
                    && provider != pkg {
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
            let cycles = self.find_cycles();
            if !cycles.is_empty() {
                let cycle_descriptions: Vec<String> = cycles
                    .iter()
                    .map(|c| format!("[{}]", c.join(" <-> ")))
                    .collect();
                return Err(eyre!(
                    "Circular dependency detected! Identified cyclic loops: {}. (Use cycle-breaker or staged build to resolve automatically via base chroot fallback)",
                    cycle_descriptions.join(", ")
                ));
            }

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

    /// Finds all Strongly Connected Components (SCCs) in the graph using Tarjan's algorithm.
    pub fn find_strongly_connected_components(&self) -> Vec<Vec<String>> {
        struct Tarjan<'a> {
            dependencies: &'a HashMap<String, HashSet<String>>,
            index: usize,
            indices: HashMap<String, usize>,
            lowlinks: HashMap<String, usize>,
            on_stack: HashSet<String>,
            stack: Vec<String>,
            sccs: Vec<Vec<String>>,
        }

        impl<'a> Tarjan<'a> {
            fn strongconnect(&mut self, node: &str) {
                self.indices.insert(node.to_string(), self.index);
                self.lowlinks.insert(node.to_string(), self.index);
                self.index += 1;
                self.stack.push(node.to_string());
                self.on_stack.insert(node.to_string());

                if let Some(neighbors) = self.dependencies.get(node) {
                    for neighbor in neighbors {
                        if !self.indices.contains_key(neighbor) {
                            self.strongconnect(neighbor);
                            let neighbor_low = self.lowlinks[neighbor];
                            let node_low = self.lowlinks.get_mut(node).unwrap();
                            if neighbor_low < *node_low {
                                *node_low = neighbor_low;
                            }
                        } else if self.on_stack.contains(neighbor) {
                            let neighbor_idx = self.indices[neighbor];
                            let node_low = self.lowlinks.get_mut(node).unwrap();
                            if neighbor_idx < *node_low {
                                *node_low = neighbor_idx;
                            }
                        }
                    }
                }

                if self.lowlinks[node] == self.indices[node] {
                    let mut scc = Vec::new();
                    while let Some(w) = self.stack.pop() {
                        self.on_stack.remove(&w);
                        let done = w == node;
                        scc.push(w);
                        if done {
                            break;
                        }
                    }
                    self.sccs.push(scc);
                }
            }
        }

        let mut tarjan = Tarjan {
            dependencies: &self.dependencies,
            index: 0,
            indices: HashMap::new(),
            lowlinks: HashMap::new(),
            on_stack: HashSet::new(),
            stack: Vec::new(),
            sccs: Vec::new(),
        };

        for node in self.packages.keys() {
            if !tarjan.indices.contains_key(node) {
                tarjan.strongconnect(node);
            }
        }

        tarjan.sccs
    }

    /// Identifies all circular dependency components (SCCs with size > 1 or self-loops).
    pub fn find_cycles(&self) -> Vec<Vec<String>> {
        let sccs = self.find_strongly_connected_components();
        let mut cycles = Vec::new();
        for mut scc in sccs {
            if scc.len() > 1 {
                scc.sort();
                cycles.push(scc);
            } else if let Some(single) = scc.first()
                && let Some(deps) = self.dependencies.get(single)
                && deps.contains(single)
            {
                cycles.push(scc);
            }
        }
        cycles.sort_by_key(|c| c.first().cloned().unwrap_or_default());
        cycles
    }

    /// Iteratively detects and breaks cycles in the dependency graph using base chroot fallback.
    ///
    /// For each circular dependency loop, one dependency edge is severed so that
    /// Kahn's algorithm can successfully schedule all compilation layers.
    /// The severed dependency will be satisfied via the base buildroot/chroot upstream repository.
    pub fn break_cycles(&mut self) -> Vec<BrokenCycleEdge> {
        let mut broken_edges = Vec::new();

        loop {
            let cycles = self.find_cycles();
            if cycles.is_empty() {
                break;
            }

            let mut broke_any = false;
            for cycle in cycles {
                let cycle_set: HashSet<String> = cycle.into_iter().collect();
                let mut edge_to_remove = None;
                for consumer in &cycle_set {
                    if let Some(deps) = self.dependencies.get(consumer) {
                        for prereq in deps {
                            if cycle_set.contains(prereq) {
                                edge_to_remove = Some((consumer.clone(), prereq.clone()));
                                break;
                            }
                        }
                    }
                    if edge_to_remove.is_some() {
                        break;
                    }
                }

                if let Some((consumer, prereq)) = edge_to_remove {
                    if let Some(deps) = self.dependencies.get_mut(&consumer) {
                        deps.remove(&prereq);
                    }
                    if let Some(deps) = self.dependents.get_mut(&prereq) {
                        deps.remove(&consumer);
                    }
                    info!(
                        "[Cycle Breaker] Severed cyclic edge '{} -> {}' via base chroot fallback",
                        consumer, prereq
                    );
                    broken_edges.push(BrokenCycleEdge {
                        consumer,
                        prerequisite: prereq,
                    });
                    broke_any = true;
                    break;
                }
            }

            if !broke_any {
                break;
            }
        }

        broken_edges
    }

    /// Computes compilation layers with automatic circular dependency resolution.
    ///
    /// If circular dependencies are present, breaks them by falling back to base chroot
    /// packages, records the severed edges, and returns the computed parallel layers.
    pub fn compute_layers_with_cycle_breaker(&mut self) -> Result<(Vec<CompilationLayer>, Vec<BrokenCycleEdge>)> {
        let broken = self.break_cycles();
        let layers = self.compute_layers()?;
        Ok((layers, broken))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kahn_layers_linear() {
        let mut graph = DependencyGraph::new();

        let meta_a = SpecMetadata {
            name: "pkg-a".to_string(),
            ..Default::default()
        };

        let meta_b = SpecMetadata {
            name: "pkg-b".to_string(),
            build_requires: vec!["pkg-a".to_string()],
            ..Default::default()
        };

        let meta_c = SpecMetadata {
            name: "pkg-c".to_string(),
            build_requires: vec!["pkg-b".to_string()],
            ..Default::default()
        };

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

        let meta_a = SpecMetadata {
            name: "pkg-a".to_string(),
            ..Default::default()
        };

        let meta_b = SpecMetadata {
            name: "pkg-b".to_string(),
            ..Default::default()
        };

        let meta_c = SpecMetadata {
            name: "pkg-c".to_string(),
            build_requires: vec!["pkg-a".to_string(), "pkg-b".to_string()],
            ..Default::default()
        };

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

        let meta_x = SpecMetadata {
            name: "pkg-x".to_string(),
            build_requires: vec!["pkg-y".to_string()],
            ..Default::default()
        };

        let meta_y = SpecMetadata {
            name: "pkg-y".to_string(),
            build_requires: vec!["pkg-x".to_string()],
            ..Default::default()
        };

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

    #[test]
    fn test_cycle_breaker_resolves_circular_dependencies() {
        let mut graph = DependencyGraph::new();

        let meta_x = SpecMetadata {
            name: "pkg-x".to_string(),
            build_requires: vec!["pkg-y".to_string()],
            ..Default::default()
        };

        let meta_y = SpecMetadata {
            name: "pkg-y".to_string(),
            build_requires: vec!["pkg-x".to_string()],
            ..Default::default()
        };

        graph.add_package(meta_x);
        graph.add_package(meta_y);
        graph.resolve_edges();

        let (layers, broken) = graph
            .compute_layers_with_cycle_breaker()
            .expect("Cycle breaker should resolve cycle");
        assert_eq!(broken.len(), 1);
        assert_eq!(layers.len(), 2);
        let all_packages: Vec<String> = layers.into_iter().flat_map(|l| l.packages).collect();
        assert!(all_packages.contains(&"pkg-x".to_string()));
        assert!(all_packages.contains(&"pkg-y".to_string()));
    }
}
