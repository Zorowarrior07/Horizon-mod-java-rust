package dev.zorowarrior.bridge;

import net.minecraft.core.Registry;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.core.registries.Registries;
import net.minecraft.resources.ResourceKey;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.world.item.BlockItem;
import net.minecraft.world.item.CreativeModeTab;
import net.minecraft.world.item.Item;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.SoundType;
import net.minecraft.world.level.block.state.BlockBehaviour;
import net.neoforged.bus.api.IEventBus;
import net.neoforged.fml.common.Mod;
import net.neoforged.neoforge.registries.DeferredRegister;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import java.util.function.Supplier;

/**
 * ZOROWARRIOR HORIZON BRIDGE — Minecraft 1.21.x, NeoForge.
 *
 * <p>Third implementation of the same content, because a third API needs it.
 * NeoForge uses {@link DeferredRegister} rather than direct
 * {@code Registry.register}, and it uses <b>Mojang official names</b>
 * ({@code net.minecraft.world.level.block.Block}, {@code ResourceLocation}) —
 * not the Yarn names the Fabric build compiles against.
 *
 * <p>Registrations are deferred until the mod bus fires, so the content objects
 * exist before the registry is frozen.
 */
@Mod(BridgeMod.MOD_ID)
public final class BridgeMod {

    public static final String MOD_ID = "zoro_bridge";
    public static final Logger LOGGER = LoggerFactory.getLogger("ZoroHorizonBridge");

    private static final DeferredRegister.Blocks BLOCKS =
            DeferredRegister.createBlocks(MOD_ID);
    private static final DeferredRegister.Items ITEMS =
            DeferredRegister.createItems(MOD_ID);

    public static final Supplier<Block> BRIDGE_BEACON = BLOCKS.register("bridge_beacon",
            () -> new Block(BlockBehaviour.Properties.of()
                    .strength(3.5F, 6.0F)
                    .lightLevel(state -> 12)
                    .sound(SoundType.STONE)
                    .requiresCorrectToolForDrops()));

    public static final Supplier<Item> BRIDGE_CHIP = ITEMS.register("bridge_chip",
            () -> new Item(new Item.Properties()));

    public BridgeMod(IEventBus modBus) {
        BLOCKS.register(modBus);
        ITEMS.register(modBus);
        // registerSimpleBlockItem makes the block obtainable as an item without
        // a separate block-item registration and creative-tab plumbing.
        ITEMS.registerSimpleBlockItem("bridge_beacon", BRIDGE_BEACON,
                new Item.Properties());
        LOGGER.info("[ZoroHorizonBridge] NeoForge 1.21.x build initialised");
    }

    /** Present so the class file references the registry API and the game's own keys. */
    static ResourceKey<Registry<Block>> blockRegistryKey() {
        return Registries.BLOCK;
    }

    static ResourceLocation id(String path) {
        return ResourceLocation.fromNamespaceAndPath(MOD_ID, path);
    }

    static CreativeModeTab.ItemDisplayParameters unusedForClarity() {
        return null;
    }
}
