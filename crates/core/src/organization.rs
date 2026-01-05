// Organization management

use crate::types::{Organization, OrgId, Person, PersonId, Team, TeamId};
use anyhow::Result;
use std::collections::HashMap;

/// Organization manager for validating and querying org structures
pub struct OrganizationManager {
    org: Organization,
}

impl OrganizationManager {
    pub fn new(org: Organization) -> Result<Self> {
        // Validate the organization structure
        Self::validate(&org)?;
        Ok(Self { org })
    }

    /// Validate the organization structure
    fn validate(org: &Organization) -> Result<()> {
        // Check that all team members exist as people
        for team in &org.teams {
            for member_id in &team.members {
                if !org.people.iter().any(|p| &p.id == member_id) {
                    anyhow::bail!("Team {} references non-existent person {}", team.id.0, member_id.0);
                }
            }

            // Check team lead exists
            if let Some(lead_id) = &team.lead {
                if !org.people.iter().any(|p| &p.id == lead_id) {
                    anyhow::bail!("Team {} has non-existent lead {}", team.id.0, lead_id.0);
                }
            }

            // Check parent team exists
            if let Some(parent_id) = &team.parent_team {
                if !org.teams.iter().any(|t| &t.id == parent_id) {
                    anyhow::bail!("Team {} has non-existent parent team {}", team.id.0, parent_id.0);
                }
            }
        }

        // Check that all people reference valid teams
        for person in &org.people {
            if !org.teams.iter().any(|t| t.id == person.team) {
                anyhow::bail!("Person {} references non-existent team {}", person.id.0, person.team.0);
            }

            // Check that reports_to exists
            if let Some(manager_id) = &person.reports_to {
                if !org.people.iter().any(|p| &p.id == manager_id) {
                    anyhow::bail!("Person {} reports to non-existent person {}", person.id.0, manager_id.0);
                }
            }
        }

        // Check root team exists
        if !org.teams.iter().any(|t| t.id == org.org_chart.root_team) {
            anyhow::bail!("Org chart references non-existent root team {}", org.org_chart.root_team.0);
        }

        // Check for cycles in reporting structure
        Self::check_reporting_cycles(&org.people)?;

        Ok(())
    }

    /// Check for cycles in the reporting structure
    fn check_reporting_cycles(people: &[Person]) -> Result<()> {
        let reporting: HashMap<PersonId, PersonId> = people
            .iter()
            .filter_map(|p| p.reports_to.as_ref().map(|m| (p.id.clone(), m.clone())))
            .collect();

        for person in people {
            let mut visited = std::collections::HashSet::new();
            let mut current = person.id.clone();

            while let Some(manager) = reporting.get(&current) {
                if !visited.insert(current.clone()) {
                    anyhow::bail!("Reporting cycle detected involving person {}", person.id.0);
                }
                current = manager.clone();
            }
        }

        Ok(())
    }

    /// Get a person by ID
    pub fn get_person(&self, person_id: &PersonId) -> Option<&Person> {
        self.org.people.iter().find(|p| &p.id == person_id)
    }

    /// Get a team by ID
    pub fn get_team(&self, team_id: &TeamId) -> Option<&Team> {
        self.org.teams.iter().find(|t| &t.id == team_id)
    }

    /// Get all direct reports for a person
    pub fn get_direct_reports(&self, person_id: &PersonId) -> Vec<&Person> {
        self.org
            .people
            .iter()
            .filter(|p| p.reports_to.as_ref() == Some(person_id))
            .collect()
    }

    /// Get all team members (including sub-teams)
    pub fn get_all_team_members(&self, team_id: &TeamId) -> Vec<&Person> {
        let mut members = Vec::new();
        let mut teams_to_process = vec![team_id.clone()];

        while let Some(current_team_id) = teams_to_process.pop() {
            if let Some(team) = self.get_team(&current_team_id) {
                // Add direct members
                for member_id in &team.members {
                    if let Some(person) = self.get_person(member_id) {
                        members.push(person);
                    }
                }

                // Add sub-teams to process
                for sub_team in &self.org.teams {
                    if sub_team.parent_team.as_ref() == Some(&current_team_id) {
                        teams_to_process.push(sub_team.id.clone());
                    }
                }
            }
        }

        members
    }

    /// Check if a person can approve a specific approval type
    pub fn can_approve(&self, person_id: &PersonId, approval_type: &str) -> bool {
        self.get_person(person_id)
            .map(|p| p.can_approve.contains(&approval_type.to_string()))
            .unwrap_or(false)
    }

    /// Get the chain of command from a person to the root
    pub fn get_management_chain(&self, person_id: &PersonId) -> Vec<&Person> {
        let mut chain = Vec::new();
        let mut current_id = person_id.clone();

        while let Some(person) = self.get_person(&current_id) {
            chain.push(person);
            if let Some(manager_id) = &person.reports_to {
                current_id = manager_id.clone();
            } else {
                break;
            }
        }

        chain
    }

    /// Get the organization
    pub fn organization(&self) -> &Organization {
        &self.org
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{OrgChart, RoleId};
    use chrono::Utc;

    fn create_test_org() -> Organization {
        let ceo = Person {
            id: PersonId::new("ceo"),
            name: "CEO".to_string(),
            email: "ceo@example.com".to_string(),
            role: RoleId::new("executive"),
            team: TeamId::new("executive"),
            reports_to: None,
            can_approve: vec!["all".to_string()],
        };

        let cto = Person {
            id: PersonId::new("cto"),
            name: "CTO".to_string(),
            email: "cto@example.com".to_string(),
            role: RoleId::new("executive"),
            team: TeamId::new("executive"),
            reports_to: Some(PersonId::new("ceo")),
            can_approve: vec!["technical".to_string(), "budget".to_string()],
        };

        let engineer = Person {
            id: PersonId::new("eng1"),
            name: "Engineer 1".to_string(),
            email: "eng1@example.com".to_string(),
            role: RoleId::new("engineer"),
            team: TeamId::new("engineering"),
            reports_to: Some(PersonId::new("cto")),
            can_approve: vec![],
        };

        let exec_team = Team {
            id: TeamId::new("executive"),
            name: "Executive".to_string(),
            description: "Executive team".to_string(),
            lead: Some(PersonId::new("ceo")),
            members: vec![PersonId::new("ceo"), PersonId::new("cto")],
            parent_team: None,
        };

        let eng_team = Team {
            id: TeamId::new("engineering"),
            name: "Engineering".to_string(),
            description: "Engineering team".to_string(),
            lead: Some(PersonId::new("cto")),
            members: vec![PersonId::new("eng1")],
            parent_team: Some(TeamId::new("executive")),
        };

        Organization {
            id: OrgId::new("test_org"),
            name: "Test Org".to_string(),
            description: "Test organization".to_string(),
            teams: vec![exec_team, eng_team],
            people: vec![ceo, cto, engineer],
            org_chart: OrgChart {
                root_team: TeamId::new("executive"),
                reporting_structure: [
                    (PersonId::new("cto"), PersonId::new("ceo")),
                    (PersonId::new("eng1"), PersonId::new("cto")),
                ]
                .iter()
                .cloned()
                .collect(),
            },
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn test_org_validation_success() {
        let org = create_test_org();
        let manager = OrganizationManager::new(org);
        assert!(manager.is_ok());
    }

    #[test]
    fn test_get_person() {
        let org = create_test_org();
        let manager = OrganizationManager::new(org).unwrap();

        let person = manager.get_person(&PersonId::new("cto"));
        assert!(person.is_some());
        assert_eq!(person.unwrap().name, "CTO");
    }

    #[test]
    fn test_get_direct_reports() {
        let org = create_test_org();
        let manager = OrganizationManager::new(org).unwrap();

        let reports = manager.get_direct_reports(&PersonId::new("cto"));
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].id, PersonId::new("eng1"));
    }

    #[test]
    fn test_get_all_team_members() {
        let org = create_test_org();
        let manager = OrganizationManager::new(org).unwrap();

        // Executive team includes sub-teams
        let members = manager.get_all_team_members(&TeamId::new("executive"));
        assert_eq!(members.len(), 3); // ceo, cto, eng1
    }

    #[test]
    fn test_can_approve() {
        let org = create_test_org();
        let manager = OrganizationManager::new(org).unwrap();

        assert!(manager.can_approve(&PersonId::new("ceo"), "all"));
        assert!(manager.can_approve(&PersonId::new("cto"), "technical"));
        assert!(!manager.can_approve(&PersonId::new("eng1"), "technical"));
    }

    #[test]
    fn test_management_chain() {
        let org = create_test_org();
        let manager = OrganizationManager::new(org).unwrap();

        let chain = manager.get_management_chain(&PersonId::new("eng1"));
        assert_eq!(chain.len(), 3); // eng1 -> cto -> ceo
        assert_eq!(chain[0].id, PersonId::new("eng1"));
        assert_eq!(chain[1].id, PersonId::new("cto"));
        assert_eq!(chain[2].id, PersonId::new("ceo"));
    }

    #[test]
    fn test_cycle_detection() {
        let mut org = create_test_org();

        // Create a cycle: ceo reports to eng1
        org.people[0].reports_to = Some(PersonId::new("eng1"));

        let result = OrganizationManager::new(org);
        assert!(result.is_err());
    }
}
