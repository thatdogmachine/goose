import { listRecipes, RecipeManifestResponse } from '../api';
import { Recipe } from './index';
import * as yaml from 'yaml';

export interface SaveRecipeOptions {
  name: string;
  global?: boolean; // true for global (~/.config/goose/recipes/), false for project-specific (.goose/recipes/)
}

export interface SavedRecipe {
  name: string;
  recipe: Recipe;
  isGlobal: boolean;
  lastModified: Date;
  isArchived?: boolean;
  filename: string; // The actual filename used
}

/**
 * Sanitize a recipe name to be safe for use as a filename
 */
function sanitizeRecipeName(name: string): string {
  return name.replace(/[^a-zA-Z0-9-_\s]/g, '').trim();
}

/**
 * Parse a lastModified value that could be a string or Date
 */
function parseLastModified(val: string | Date): Date {
  return val instanceof Date ? val : new Date(val);
}

/**
 * Get the storage directory path for recipes
 */
export function getStorageDirectory(isGlobal: boolean): string {
  if (isGlobal) {
    return '~/.config/goose/recipes';
  } else {
    const projectDir = window.appConfig.get('GOOSE_WORKING_DIR') as string;
    // Fallback to (broken) relative path if projectDir is not available for some reason
    return projectDir ? `${projectDir}/.goose/recipes` : '.goose/recipes';
  }
}

/**
 * Get the file path for a recipe based on its name
 */
function getRecipeFilePath(recipeName: string, isGlobal: boolean): string {
  const dir = getStorageDirectory(isGlobal);
  return `${dir}/${recipeName}.yaml`;
}

/**
 * Load recipe from file
 */
async function loadRecipeFromFile(
  recipeName: string,
  isGlobal: boolean
): Promise<SavedRecipe | null> {
  const filePath = getRecipeFilePath(recipeName, isGlobal);

  try {
    const result = await window.electron.readFile(filePath);
    if (!result.found || result.error) {
      return null;
    }

    const recipeData = yaml.parse(result.file) as SavedRecipe;

    // Convert lastModified string to Date if needed
    recipeData.lastModified = parseLastModified(recipeData.lastModified);

    return {
      ...recipeData,
      isGlobal: isGlobal,
      filename: recipeName,
    };
  } catch (error) {
    console.warn(`Failed to load recipe from ${filePath}:`, error);
    return null;
  }
}

/**
 * Save recipe to file
 */
async function saveRecipeToFile(recipe: SavedRecipe): Promise<boolean> {
  const filePath = getRecipeFilePath(recipe.name, recipe.isGlobal);

  // Ensure directory exists
  const dirPath = getStorageDirectory(recipe.isGlobal);
  await window.electron.ensureDirectory(dirPath);

  // Convert to YAML and save
  const yamlContent = yaml.stringify(recipe);
  return await window.electron.writeFile(filePath, yamlContent);
}
/**
 * Save a recipe to a file using IPC.
 */
export async function saveRecipe(recipe: Recipe, options: SaveRecipeOptions): Promise<string> {
  const { name, global = true } = options;

  // Sanitize name
  const sanitizedName = sanitizeRecipeName(name);
  if (!sanitizedName) {
    throw new Error('Invalid recipe name');
  }

  // Validate recipe has required fields
  if (!recipe.title || !recipe.description) {
    throw new Error('Recipe is missing required fields (title, description)');
  }

  if (!recipe.instructions && !recipe.prompt) {
    throw new Error('Recipe must have either instructions or prompt');
  }

  try {
    // Create saved recipe object
    const savedRecipe: SavedRecipe = {
      name: sanitizedName,
      filename: sanitizedName,
      recipe: recipe,
      isGlobal: global,
      lastModified: new Date(),
      isArchived: false,
    };

    // Save to file
    const success = await saveRecipeToFile(savedRecipe);

    if (!success) {
      throw new Error('Failed to save recipe file');
    }

    // Return identifier for the saved recipe
    return `${global ? 'global' : 'local'}:${sanitizedName}`;
  } catch (error) {
    throw new Error(
      `Failed to save recipe: ${error instanceof Error ? error.message : 'Unknown error'}`
    );
  }
}

/**
 * Load a recipe by name from file.
 */
export async function loadRecipe(recipeName: string, isGlobal: boolean): Promise<Recipe> {
  try {
    const savedRecipe = await loadRecipeFromFile(recipeName, isGlobal);

    if (!savedRecipe) {
      throw new Error('Recipe not found');
    }

    // Validate the loaded recipe has required fields
    if (!savedRecipe.recipe.title || !savedRecipe.recipe.description) {
      throw new Error('Loaded recipe is missing required fields');
    }

    if (!savedRecipe.recipe.instructions && !savedRecipe.recipe.prompt) {
      throw new Error('Loaded recipe must have either instructions or prompt');
    }

    return savedRecipe.recipe;
  } catch (error) {
    throw new Error(
      `Failed to load recipe: ${error instanceof Error ? error.message : 'Unknown error'}`
    );
  }
}

export async function listSavedRecipes(): Promise<RecipeManifestResponse[]> {
  try {
    const listRecipeResponse = await listRecipes();
    return listRecipeResponse?.data?.recipe_manifest_responses ?? [];
  } catch (error) {
    console.warn('Failed to list saved recipes:', error);
    return [];
  }
}

export function convertToLocaleDateString(lastModified: string): string {
  if (lastModified) {
    return parseLastModified(lastModified).toLocaleDateString();
  }
  return '';
}

/**
 * Generate a suggested filename for a recipe based on its title.
 *
 * @param recipe The recipe to generate a filename for
 * @returns A sanitized filename suitable for use as a recipe name
 */
export function generateRecipeFilename(recipe: Recipe): string {
  const baseName = recipe.title
    .toLowerCase()
    .replace(/[^a-zA-Z0-9\s-]/g, '')
    .replace(/\s+/g, '-')
    .trim();

  return baseName || 'untitled-recipe';
}
